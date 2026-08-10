use crate::common::{PagedResponse, parse_query};
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    extract::{Path, Query as AxumQuery},
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, TimeZone, Utc};
use flowable_engine::engine::management_service::RuntimeJobFamily;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_engine::persistence::{DbParams, DbValue, StorageError};
use flowable_platform_bootstrap::{
    CertifiedTopologyProfile, DirectoryProviderKind, DirectorySupportContract,
    EnterpriseAdapterFamily, EnterpriseAdapterSupportContract, EnterpriseSupportKind,
    OperationsExposureKind, OperationsObjectFamilyBreadth, OperationsSupportContract,
    RuntimeEmbeddingContract, RuntimeEmbeddingMode, RuntimeEmbeddingProfile,
    TopologyCertificationContract,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[derive(Clone)]
pub struct ManagementApiState {
    pub runtime_embedding_contract: RuntimeEmbeddingContract,
    pub enterprise_support_contracts: Vec<EnterpriseAdapterSupportContract>,
    pub enterprise_support_statement: String,
    pub directory_support_contract: DirectorySupportContract,
    pub operations_support_contract: OperationsSupportContract,
    pub topology_certification_contract: TopologyCertificationContract,
}

pub fn router(state: Arc<ManagementApiState>) -> Router {
    Router::new()
        .route("/management/directory/support", get(get_directory_support))
        .route(
            "/management/directory/reconcile",
            get(get_directory_reconcile_report).post(post_directory_reconcile),
        )
        .route(
            "/management/operations/support",
            get(get_operations_support),
        )
        .route("/management/platform/support", get(get_platform_support))
        .route(
            "/management/platform/topology-certification",
            get(get_topology_certification),
        )
        .route("/management/jmx/runtime", get(get_jmx_runtime))
        .route(
            "/management/jmx/connector-descriptor",
            get(get_jmx_connector_descriptor),
        )
        .route(
            "/management/jmx/mbean-registry",
            get(get_jmx_mbean_registry),
        )
        .route(
            "/management/jmx/operations-bus",
            get(get_jmx_operations_bus),
        )
        .route(
            "/management/jmx/runtime-ledger",
            get(get_jmx_runtime_ledger),
        )
        .route("/management/jmx/timer-ledger", get(get_jmx_timer_ledger))
        .route(
            "/management/operations/topology",
            get(get_operations_topology),
        )
        .layer(Extension(state))
}

pub fn engine_router() -> Router {
    Router::new()
        .route("/management/engine", get(get_engine))
        .route("/management/properties", get(get_properties))
        .route(
            "/management/engine-properties",
            get(get_engine_properties).post(create_engine_property),
        )
        .route(
            "/management/engine-properties/:engine_property",
            get(get_engine_property)
                .put(update_engine_property)
                .delete(delete_engine_property),
        )
        .route("/management/tables", get(list_tables))
        .route("/management/tables/:table_name", get(get_table))
        .route(
            "/management/tables/:table_name/columns",
            get(get_table_columns),
        )
        .route("/management/tables/:table_name/data", get(get_table_data))
        .route("/management/jobs", get(list_jobs))
        .route(
            "/management/jobs/:job_id",
            get(get_job).post(post_job).delete(delete_job),
        )
        .route(
            "/management/jobs/:job_id/exception-stacktrace",
            get(get_job_exception_stacktrace),
        )
        .route("/management/timer-jobs", get(list_timer_jobs))
        .route(
            "/management/timer-jobs/:job_id",
            get(get_timer_job)
                .post(post_timer_job)
                .delete(delete_timer_job),
        )
        .route(
            "/management/timer-jobs/:job_id/exception-stacktrace",
            get(get_timer_job_exception_stacktrace),
        )
        .route(
            "/management/deadletter-jobs",
            get(list_deadletter_jobs).post(post_deadletter_jobs_bulk),
        )
        .route(
            "/management/deadletter-jobs/:job_id",
            get(get_deadletter_job)
                .post(post_deadletter_job)
                .delete(delete_deadletter_job),
        )
        .route(
            "/management/deadletter-jobs/:job_id/exception-stacktrace",
            get(get_deadletter_job_exception_stacktrace),
        )
        .route("/management/history-jobs", get(list_history_jobs))
        .route(
            "/management/history-jobs/:job_id",
            get(get_history_job)
                .post(post_history_job)
                .delete(delete_history_job),
        )
        .route("/management/suspended-jobs", get(list_suspended_jobs))
        .route(
            "/management/suspended-jobs/:job_id",
            get(get_suspended_job)
                .post(post_suspended_job)
                .delete(delete_suspended_job),
        )
        .route(
            "/management/suspended-jobs/:job_id/exception-stacktrace",
            get(get_suspended_job_exception_stacktrace),
        )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectorySupportResponse {
    provider: String,
    sync_on_bootstrap: bool,
    transport: String,
    auth_mode: String,
    deployment_mode: String,
    conflict_policy: String,
    filter_breadth: String,
    imported_user_count: usize,
    imported_group_count: usize,
    imported_membership_count: usize,
    runtime_user_read_enabled: bool,
    runtime_group_read_enabled: bool,
    runtime_membership_read_enabled: bool,
    runtime_user_write_enabled: bool,
    runtime_group_write_enabled: bool,
    runtime_membership_write_enabled: bool,
    runtime_reconcile_enabled: bool,
    runtime_bidirectional_sync_enabled: bool,
    support_statement: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationsSupportResponse {
    exposure: String,
    management_api_enabled: bool,
    runtime_ledger_enabled: bool,
    timer_ledger_enabled: bool,
    topology_ledger_enabled: bool,
    native_compatible_connector_enabled: bool,
    mbean_registry_enabled: bool,
    operations_bus_enabled: bool,
    object_family_breadth: String,
    support_statement: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformSupportResponse {
    embedding: EmbeddingSupportResponse,
    enterprise: EnterpriseSupportResponse,
    directory: DirectorySupportResponse,
    operations: OperationsSupportResponse,
    topology_certification: TopologyCertificationResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddingSupportResponse {
    mode: String,
    profile: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnterpriseSupportResponse {
    adapter_count: usize,
    adapters: Vec<String>,
    support_kinds: Vec<String>,
    support_statement: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TopologyCertificationResponse {
    profile: String,
    ingress: String,
    packaging: String,
    startup_certified: bool,
    auth_certified: bool,
    cutover_certified: bool,
    rollback_certified: bool,
    recovery_certified: bool,
    supported_historical_ingress: Vec<String>,
    support_statement: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxRuntimeResponse {
    exposure: String,
    management_api_enabled: bool,
    directory_provider: String,
    identity: IdentityCounts,
    engine_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxConnectorDescriptorResponse {
    connector_family: String,
    exposure: String,
    management_api_enabled: bool,
    transport: String,
    simulated_rmi_transport: bool,
    endpoint_prefix: String,
    engine_name: String,
    embedding: EmbeddingSupportResponse,
    directory_provider: String,
    enterprise_adapters: Vec<String>,
    object_family_breadth: String,
    support_statement: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxMbeanRegistryResponse {
    domain: String,
    engine_name: String,
    exposure: String,
    object_family_breadth: String,
    mbean_count: usize,
    mbeans: Vec<JmxMbeanDescriptorResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxMbeanDescriptorResponse {
    object_name: String,
    kind: String,
    path: String,
    attributes: Vec<String>,
    operations: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxRuntimeLedgerResponse {
    engine_name: String,
    exposure: String,
    process_instance_count: usize,
    execution_count: usize,
    task_count: usize,
    variable_count: usize,
    event_subscription_count: usize,
    identity: IdentityCounts,
    directory_provider: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxTimerLedgerResponse {
    exposure: String,
    management_api_enabled: bool,
    total_timer_job_count: usize,
    active_timer_job_count: usize,
    locked_timer_job_count: usize,
    coordinator: TimerCoordinatorResponse,
    nodes: Vec<TimerNodeResponse>,
    metrics: TimerMetricsResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationsTopologyResponse {
    engine_name: String,
    embedding: EmbeddingSupportResponse,
    enterprise: EnterpriseSupportResponse,
    directory_provider: String,
    directory_runtime_reads: DirectoryRuntimeReadsResponse,
    operations: OperationsSupportResponse,
    coordinator: TimerCoordinatorResponse,
    nodes: Vec<TimerNodeResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxOperationsBusResponse {
    family: String,
    object_family_breadth: String,
    exposure: String,
    management_api_enabled: bool,
    engine_name: String,
    connector: JmxBusConnectorReference,
    registry: JmxBusRegistryReference,
    runtime_ledger: Option<JmxRuntimeLedgerResponse>,
    timer_ledger: Option<JmxTimerLedgerResponse>,
    topology: Option<OperationsTopologyResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxBusConnectorReference {
    path: String,
    connector_family: String,
    transport: String,
    simulated_rmi_transport: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmxBusRegistryReference {
    path: String,
    domain: String,
    mbean_count: usize,
}

#[derive(Serialize)]
struct IdentityCounts {
    users: usize,
    groups: usize,
    tokens: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryRuntimeReadsResponse {
    user: bool,
    group: bool,
    membership: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryReconcileResponse {
    provider: String,
    mode: String,
    supported: bool,
    applied: bool,
    live_user_count: usize,
    live_group_count: usize,
    live_membership_count: usize,
    shadowed_user_ids: Vec<String>,
    shadowed_group_ids: Vec<String>,
    shadowed_memberships: Vec<MembershipKeyResponse>,
    owned_only_user_ids: Vec<String>,
    owned_only_group_ids: Vec<String>,
    owned_only_memberships: Vec<MembershipKeyResponse>,
    added_users: usize,
    added_groups: usize,
    added_memberships: usize,
    removed_users: usize,
    removed_groups: usize,
    removed_memberships: usize,
}

#[derive(Default, Deserialize)]
struct DirectoryReconcileParams {
    mode: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirectoryReconcileMode {
    LiveWins,
    OwnedToLive,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MembershipKeyResponse {
    user_id: String,
    group_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TimerCoordinatorResponse {
    leader_node_id: String,
    fencing_token: i64,
    lease_expiry_time: i64,
    status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TimerNodeResponse {
    node_id: String,
    worker_type: String,
    last_heartbeat: i64,
    status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TimerMetricsResponse {
    acquire_attempts: usize,
    acquire_conflicts: usize,
    jobs_acquired: usize,
    last_acquire_batch_size: usize,
    renew_successes: usize,
    renew_misses: usize,
    expired_lease_recoveries: usize,
    execute_count_runtime_job: usize,
    execute_count_process_start: usize,
    execute_count_event_subprocess: usize,
    execute_failures: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineInfoResponse {
    name: String,
    version: String,
    resource_url: Option<String>,
    exception: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementPropertiesResponse {
    engine_name: String,
    version: String,
    schema_table_count: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnginePropertyResponse {
    name: String,
    value: String,
    revision: i32,
}

#[derive(Deserialize)]
struct EnginePropertyRequest {
    name: Option<String>,
    value: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableResponse {
    name: String,
    url: String,
    count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableColumnResponse {
    name: String,
    column_type: String,
    nullable: bool,
    primary_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableMetaDataResponse {
    table_name: String,
    column_names: Vec<String>,
    column_types: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TableDataQuery {
    start: usize,
    size: Option<usize>,
    order_ascending_column: Option<String>,
    order_descending_column: Option<String>,
}

#[derive(Serialize)]
struct TableDataResponse {
    start: usize,
    size: usize,
    total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<String>,
    data: Vec<Value>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ManagementListQuery {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    process_instance_id: Option<String>,
    process_definition_id: Option<String>,
    execution_id: Option<String>,
    element_id: Option<String>,
    element_name: Option<String>,
    handler_type: Option<String>,
    handler_types: Option<String>,
    /// Engine-query dimensions (also accepted on management REST for host tooling).
    /// Java BPMN management REST historically omits several of these; the direct
    /// engine query remains the full surface.
    category: Option<String>,
    category_like: Option<String>,
    scope_id: Option<String>,
    sub_scope_id: Option<String>,
    scope_type: Option<String>,
    scope_definition_id: Option<String>,
    case_definition_key: Option<String>,
    correlation_id: Option<String>,
    external_workers: Option<bool>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: Option<bool>,
    without_process_instance_id: Option<bool>,
    without_scope_id: Option<bool>,
    without_scope_type: Option<bool>,
    with_retries_left: Option<bool>,
    no_retries_left: Option<bool>,
    executable: Option<bool>,
    timers_only: Option<bool>,
    messages_only: Option<bool>,
    due_before: Option<String>,
    due_after: Option<String>,
    with_exception: Option<bool>,
    without_exception: Option<bool>,
    exception_message: Option<String>,
    locked: Option<bool>,
    unlocked: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

/// Java `PaginateListUtil` defaults the page size to 10 for job-family
/// collections.
const MANAGEMENT_JOB_DEFAULT_PAGE_SIZE: usize = 10;

/// The management job families, mirroring the Java collection resources.
#[derive(Clone, Copy, PartialEq, Eq)]
enum JobFamily {
    Executable,
    Timer,
    Deadletter,
    Suspended,
    History,
}

impl JobFamily {
    /// Fallback job type used by the response mappers of this family.
    fn default_job_type(&self) -> &'static str {
        match self {
            JobFamily::Executable => "executable",
            JobFamily::Timer => "timer",
            JobFamily::Deadletter => "deadletter",
            JobFamily::Suspended => "suspended",
            JobFamily::History => "history",
        }
    }

    /// Java sort whitelist: JobQueryProperties for the runtime families,
    /// HistoryJobQueryProperties for the history family.
    fn allowed_sorts(&self) -> &'static [&'static str] {
        match self {
            // Java HistoryJobQueryProperties: id / retries / tenantId only.
            JobFamily::History => &["id", "retries", "tenantId"],
            _ => &[
                "id",
                "dueDate",
                "createTime",
                "executionId",
                "processInstanceId",
                "retries",
                "tenantId",
            ],
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct BulkDeadletterJobActionRequest {
    action: Option<String>,
    job_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementJobResponse {
    id: String,
    url: String,
    job_type: String,
    correlation_id: Option<String>,
    process_instance_id: Option<String>,
    process_instance_url: Option<String>,
    process_definition_id: Option<String>,
    process_definition_url: Option<String>,
    execution_id: Option<String>,
    execution_url: Option<String>,
    element_id: Option<String>,
    element_name: Option<String>,
    handler_type: Option<String>,
    lock_owner: Option<String>,
    due_date: Option<String>,
    create_time: Option<String>,
    lock_expiration_time: Option<String>,
    retries: i32,
    exception_message: Option<String>,
    tenant_id: Option<String>,
}

/// Java `HistoryJobResponse` (RestResponseFactory.createHistoryJobResponse).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementHistoryJobResponse {
    id: String,
    url: String,
    scope_type: Option<String>,
    retries: i32,
    exception_message: Option<String>,
    job_handler_type: Option<String>,
    job_handler_configuration: Option<String>,
    advanced_job_handler_configuration: Option<String>,
    tenant_id: Option<String>,
    custom_values: Option<String>,
    create_time: Option<String>,
    lock_owner: Option<String>,
    lock_expiration_time: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ManagementJobActionRequest {
    action: Option<String>,
    retries: Option<i32>,
    exception_message: Option<String>,
    delete_reason: Option<String>,
    due_date: Option<String>,
    time_date: Option<String>,
    time_duration: Option<String>,
    time_cycle: Option<String>,
    end_date: Option<String>,
    calendar_name: Option<String>,
}

async fn get_directory_support(
    Extension(state): Extension<Arc<ManagementApiState>>,
) -> Json<DirectorySupportResponse> {
    Json(directory_support_response(
        &state.directory_support_contract,
    ))
}

async fn get_directory_reconcile_report(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    AxumQuery(params): AxumQuery<DirectoryReconcileParams>,
) -> Result<Json<DirectoryReconcileResponse>, ApiError> {
    let mode = parse_directory_reconcile_mode(params.mode.as_deref())?;
    let report = directory_reconcile_response(&state, &engine, &directory_state, false, mode)?;
    Ok(Json(report))
}

async fn post_directory_reconcile(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    AxumQuery(params): AxumQuery<DirectoryReconcileParams>,
) -> Result<Json<DirectoryReconcileResponse>, ApiError> {
    let mode = parse_directory_reconcile_mode(params.mode.as_deref())?;
    let report = directory_reconcile_response(&state, &engine, &directory_state, true, mode)?;
    Ok(Json(report))
}

async fn get_operations_support(
    Extension(state): Extension<Arc<ManagementApiState>>,
) -> Json<OperationsSupportResponse> {
    Json(operations_support_response(
        &state.operations_support_contract,
    ))
}

async fn get_platform_support(
    Extension(state): Extension<Arc<ManagementApiState>>,
) -> Json<PlatformSupportResponse> {
    Json(PlatformSupportResponse {
        embedding: EmbeddingSupportResponse {
            mode: runtime_embedding_mode_name(state.runtime_embedding_contract.mode),
            profile: runtime_embedding_profile_name(state.runtime_embedding_contract.profile),
        },
        enterprise: EnterpriseSupportResponse {
            adapter_count: state.enterprise_support_contracts.len(),
            adapters: state
                .enterprise_support_contracts
                .iter()
                .map(|contract| enterprise_adapter_family_name(contract.family))
                .collect(),
            support_kinds: state
                .enterprise_support_contracts
                .iter()
                .map(|contract| enterprise_support_kind_name(contract.support_kind))
                .collect(),
            support_statement: state.enterprise_support_statement.clone(),
        },
        directory: directory_support_response(&state.directory_support_contract),
        operations: operations_support_response(&state.operations_support_contract),
        topology_certification: topology_certification_response(
            &state.topology_certification_contract,
        ),
    })
}

async fn get_topology_certification(
    Extension(state): Extension<Arc<ManagementApiState>>,
) -> Json<TopologyCertificationResponse> {
    Json(topology_certification_response(
        &state.topology_certification_contract,
    ))
}

async fn get_jmx_runtime(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
) -> Result<Json<JmxRuntimeResponse>, ApiError> {
    Ok(Json(jmx_runtime_response(
        &state,
        &engine,
        &directory_state,
    )?))
}

async fn get_jmx_connector_descriptor(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<JmxConnectorDescriptorResponse>, ApiError> {
    Ok(Json(jmx_connector_descriptor_response(&state, &engine)?))
}

async fn get_jmx_mbean_registry(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<JmxMbeanRegistryResponse>, ApiError> {
    Ok(Json(jmx_mbean_registry_response(&state, &engine)?))
}

async fn get_jmx_operations_bus(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
) -> Result<Json<JmxOperationsBusResponse>, ApiError> {
    Ok(Json(jmx_operations_bus_response(
        &state,
        &engine,
        &directory_state,
    )?))
}

async fn get_jmx_runtime_ledger(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
) -> Result<Json<JmxRuntimeLedgerResponse>, ApiError> {
    Ok(Json(jmx_runtime_ledger_response(
        &state,
        &engine,
        &directory_state,
    )?))
}

async fn get_jmx_timer_ledger(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<JmxTimerLedgerResponse>, ApiError> {
    Ok(Json(jmx_timer_ledger_response(&state, &engine)?))
}

async fn get_operations_topology(
    Extension(state): Extension<Arc<ManagementApiState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<OperationsTopologyResponse>, ApiError> {
    Ok(Json(operations_topology_response(&state, &engine)?))
}

async fn get_engine(Extension(engine): Extension<Arc<ProcessEngine>>) -> Json<EngineInfoResponse> {
    Json(EngineInfoResponse {
        name: engine.get_name().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        resource_url: None,
        exception: None,
    })
}

async fn get_properties(
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<ManagementPropertiesResponse>, ApiError> {
    Ok(Json(ManagementPropertiesResponse {
        engine_name: engine.get_name().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        schema_table_count: table_names(&engine)?.len(),
    }))
}

async fn get_engine_properties(
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<Vec<EnginePropertyResponse>>, ApiError> {
    engine_properties(&engine).map(|properties| Json(properties.into_values().collect()))
}

async fn get_engine_property(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(engine_property): Path<String>,
) -> Result<Json<EnginePropertyResponse>, ApiError> {
    let properties = engine_properties(&engine)?;
    properties
        .get(&engine_property)
        .cloned()
        .map(Json)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Engine property '{}' does not exist",
                engine_property
            ))
        })
}

async fn create_engine_property(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(request): Json<EnginePropertyRequest>,
) -> Result<StatusCode, ApiError> {
    let property_name = required_property_name(request.name.as_deref())?;
    let property_value = request.value.unwrap_or_default();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;

    let result =
        runtime_store.create_engine_property(&property_name, &property_value, &mut session);
    match result {
        Ok(()) => {
            session
                .flush_and_commit()
                .map_err(|e| ApiError::InternalServerError(e.to_string()))?;
            Ok(StatusCode::CREATED)
        }
        Err(StorageError::DuplicateEntity { .. }) => Err(ApiError::Conflict(format!(
            "Engine property '{property_name}' already exists"
        ))),
        Err(error) => Err(ApiError::InternalServerError(error.to_string())),
    }
}

async fn update_engine_property(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(engine_property): Path<String>,
    Json(request): Json<EnginePropertyRequest>,
) -> Result<StatusCode, ApiError> {
    let property_value = request.value.unwrap_or_default();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;

    let updated = runtime_store
        .update_engine_property(&engine_property, &property_value, &mut session)
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;
    if !updated {
        return Err(ApiError::NotFound(format!(
            "Engine property '{}' does not exist",
            engine_property
        )));
    }
    session
        .flush_and_commit()
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;
    Ok(StatusCode::OK)
}

async fn delete_engine_property(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(engine_property): Path<String>,
) -> Result<StatusCode, ApiError> {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;

    let deleted = runtime_store
        .delete_engine_property(&engine_property, &mut session)
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound(format!(
            "Engine property '{}' does not exist",
            engine_property
        )));
    }
    session
        .flush_and_commit()
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_tables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<Vec<TableResponse>>, ApiError> {
    let mut tables = table_names(&engine)?
        .into_iter()
        .map(|name| {
            let count = count_table_rows(&engine, &name).unwrap_or_default();
            table_response(name, count)
        })
        .collect::<Vec<_>>();
    tables.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Json(tables))
}

async fn get_table(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(table_name): Path<String>,
) -> Result<Json<TableResponse>, ApiError> {
    ensure_table_exists(&engine, &table_name)?;
    let count = count_table_rows(&engine, &table_name)?;
    Ok(Json(table_response(table_name, count)))
}

async fn get_table_columns(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(table_name): Path<String>,
) -> Result<Json<TableMetaDataResponse>, ApiError> {
    ensure_table_exists(&engine, &table_name)?;
    let columns = table_columns(&engine, &table_name)?;
    Ok(Json(TableMetaDataResponse {
        table_name,
        column_names: columns.iter().map(|column| column.name.clone()).collect(),
        column_types: columns
            .into_iter()
            .map(|column| column.column_type)
            .collect(),
    }))
}

async fn get_table_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(table_name): Path<String>,
    uri: Uri,
) -> Result<Json<TableDataResponse>, ApiError> {
    ensure_table_exists(&engine, &table_name)?;
    let query: TableDataQuery = parse_query(&uri)?;
    let sort = table_data_sort(&engine, &table_name, &query)?;
    let rows = table_data(&engine, &table_name, sort.as_ref())?;
    Ok(Json(paginate_table_data(rows, &query, sort)))
}

async fn list_jobs(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ManagementJobResponse>>, ApiError> {
    let query: ManagementListQuery = parse_query(&uri)?;
    list_management_jobs(&engine, &query, JobFamily::Executable, executable_job_to_management_job)
}

async fn get_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Json<ManagementJobResponse>, ApiError> {
    let job = engine
        .get_management_service()
        .find_executable_job_by_id(&job_id)
        .ok_or_else(|| ApiError::NotFound(format!("Job '{}' not found", job_id)))?;
    Ok(Json(executable_job_to_management_job(&engine, job)))
}

async fn delete_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_management_service()
        .delete_job(&job_id)
        .map_err(job_mutation_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
    Json(request): Json<ManagementJobActionRequest>,
) -> Result<StatusCode, ApiError> {
    match required_action(&request)?.as_str() {
        "execute" => {
            // Java ExecuteJobCmd: generic handler path for any executable job
            // (async continuation, timer-moved-to-executable, …).
            engine
                .get_management_service()
                .execute_job(&job_id)
                .map_err(job_execute_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        "move" | "moveToDeadLetterJob" | "move-to-deadletter-job" => {
            engine
                .get_management_service()
                .move_job_to_deadletter_job_with_fields(
                    &job_id,
                    request.exception_message,
                    request.delete_reason,
                )
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        "setRetries" | "set-retries" | "retry" | "retries" => {
            let retries = request
                .retries
                .ok_or_else(|| ApiError::bad_request("Field 'retries' is required"))?;
            engine
                .get_management_service()
                .set_job_retries(&job_id, retries)
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        action => Err(unsupported_job_action(
            "jobs",
            action,
            &[
                "execute",
                "move",
                "moveToDeadLetterJob",
                "setRetries",
                "retry",
            ],
        )),
    }
}

async fn get_job_exception_stacktrace(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    // Java `JobBaseResource.getJobById` only queries the executable job table;
    // ids from the timer/deadletter/suspended/history families are 404 here.
    let job = engine
        .get_management_service()
        .find_executable_job_by_id(&job_id)
        .ok_or_else(|| ApiError::NotFound(format!("Job '{}' not found", job_id)))?;
    job_stacktrace_response("Job", job.timer_job_id, job.error_details)
}

async fn list_timer_jobs(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ManagementJobResponse>>, ApiError> {
    let query: ManagementListQuery = parse_query(&uri)?;
    list_management_jobs(&engine, &query, JobFamily::Timer, timer_job_to_management_job)
}

async fn get_timer_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Json<ManagementJobResponse>, ApiError> {
    engine
        .get_management_service()
        .find_timer_job_by_id(&job_id)
        .map(|job| timer_job_to_management_job(&engine, job))
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("Timer job '{}' not found", job_id)))
}

async fn delete_timer_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_management_service()
        .delete_timer_job(&job_id)
        .map_err(job_mutation_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_timer_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
    Json(request): Json<ManagementJobActionRequest>,
) -> Result<StatusCode, ApiError> {
    match required_action(&request)?.as_str() {
        "execute" => {
            // Same generic execute path as executable jobs; family lookup only
            // restricts which table/id is accepted.
            engine
                .get_management_service()
                .execute_timer_job(&job_id)
                .map_err(job_execute_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        "move" | "moveToExecutableJob" | "move-to-executable-job" => {
            engine
                .get_management_service()
                .move_timer_to_executable_job(&job_id)
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        "reschedule" => {
            let time_date = request.time_date.or(request.due_date);
            engine
                .get_management_service()
                .reschedule_timer_job(
                    &job_id,
                    time_date,
                    request.time_duration,
                    request.time_cycle,
                    request.end_date,
                    request.calendar_name,
                )
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        "setRetries" | "set-retries" | "retry" | "retries" => {
            let retries = request
                .retries
                .ok_or_else(|| ApiError::bad_request("Field 'retries' is required"))?;
            engine
                .get_management_service()
                .set_job_retries(&job_id, retries)
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        action => Err(unsupported_job_action(
            "timer-jobs",
            action,
            &[
                "execute",
                "move",
                "moveToExecutableJob",
                "reschedule",
                "setRetries",
                "retry",
            ],
        )),
    }
}

async fn get_timer_job_exception_stacktrace(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    let job = engine
        .get_management_service()
        .find_timer_job_by_id(&job_id)
        .ok_or_else(|| ApiError::NotFound(format!("Timer job '{}' not found", job_id)))?;
    job_stacktrace_response("Timer job", job.timer_job_id, job.error_details)
}

async fn list_deadletter_jobs(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ManagementJobResponse>>, ApiError> {
    let query: ManagementListQuery = parse_query(&uri)?;
    list_management_jobs(
        &engine,
        &query,
        JobFamily::Deadletter,
        deadletter_job_to_management_job,
    )
}

/// Java `JobCollectionResource.executeDeadLetterJobAction`: bulk move of
/// deadletter jobs, accepting `move` and `moveToHistoryJob` actions.
async fn post_deadletter_jobs_bulk(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(request): Json<BulkDeadletterJobActionRequest>,
) -> Result<StatusCode, ApiError> {
    let action = request.action.as_deref().unwrap_or("");
    if action != "move" && action != "moveToHistoryJob" {
        return Err(ApiError::bad_request(
            "Invalid action, only 'move' or 'moveToHistoryJob' is supported.",
        ));
    }

    let management_service = engine.get_management_service();
    let existing_ids = management_service
        .list_deadletter_jobs()
        .into_iter()
        .map(|job| job.timer_job_id)
        .collect::<BTreeSet<_>>();
    let mut missing_ids: Vec<String> = Vec::new();
    for job_id in &request.job_ids {
        if !existing_ids.contains(job_id) && !missing_ids.contains(job_id) {
            missing_ids.push(job_id.clone());
        }
    }
    if !missing_ids.is_empty() {
        return Err(ApiError::NotFound(format!(
            "Could not find a dead letter job(s) with id(s) {{{}}}",
            missing_ids.join(",")
        )));
    }

    if action == "move" {
        let retries = engine.get_config().async_executor.number_of_retries;
        management_service
            .bulk_move_deadletter_jobs(&request.job_ids, retries)
            .map_err(job_mutation_error)?;
    } else {
        // Java bulk moveToHistoryJob uses asyncHistoryExecutorNumberOfRetries.
        let retries = engine.get_config().async_history.number_of_retries;
        management_service
            .bulk_move_deadletter_jobs_to_history_jobs(&request.job_ids, retries)
            .map_err(job_mutation_error)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_deadletter_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Json<ManagementJobResponse>, ApiError> {
    engine
        .get_management_service()
        .find_deadletter_job_by_id(&job_id)
        .map(|job| deadletter_job_to_management_job(&engine, job))
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("Deadletter job '{}' not found", job_id)))
}

async fn delete_deadletter_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_management_service()
        .delete_deadletter_job(&job_id)
        .map_err(job_mutation_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_deadletter_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
    Json(request): Json<ManagementJobActionRequest>,
) -> Result<StatusCode, ApiError> {
    match required_action(&request)?.as_str() {
        "move" | "moveToExecutableJob" | "move-to-executable-job" | "retry" => {
            // Java `JobResource.executeDeadLetterJobAction`: a history-origin
            // deadletter job is routed back to the history family, and the
            // default retry count comes from the engine configuration
            // (`asyncExecutorNumberOfRetries`), not a hardcoded value.
            let retries = request
                .retries
                .unwrap_or_else(|| engine.get_config().async_executor.number_of_retries);
            engine
                .get_management_service()
                .move_deadletter_job_by_origin(&job_id, retries)
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        "moveToHistoryJob" | "move-to-history-job" | "moveToHistory" | "move-to-history" => {
            // Java JobResource: moveToHistoryJob uses asyncHistoryExecutorNumberOfRetries.
            let retries = request.retries.unwrap_or_else(|| {
                engine
                    .get_config()
                    .async_history
                    .number_of_retries
            });
            engine
                .get_management_service()
                .move_deadletter_job_to_history_job(&job_id, retries)
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        "setRetries" | "set-retries" | "retries" => {
            let retries = request
                .retries
                .ok_or_else(|| ApiError::bad_request("Field 'retries' is required"))?;
            engine
                .get_management_service()
                .set_job_retries(&job_id, retries)
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        action => Err(unsupported_job_action(
            "deadletter-jobs",
            action,
            &[
                "move",
                "moveToExecutableJob",
                "retry",
                "moveToHistoryJob",
                "setRetries",
            ],
        )),
    }
}

async fn get_deadletter_job_exception_stacktrace(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    let job = engine
        .get_management_service()
        .find_deadletter_job_by_id(&job_id)
        .ok_or_else(|| ApiError::NotFound(format!("Deadletter job '{}' not found", job_id)))?;
    job_stacktrace_response("Deadletter job", job.timer_job_id, job.error_details)
}

async fn list_history_jobs(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ManagementHistoryJobResponse>>, ApiError> {
    let query: ManagementListQuery = parse_query(&uri)?;
    list_management_jobs(
        &engine,
        &query,
        JobFamily::History,
        history_job_to_management_history_job,
    )
}

async fn get_history_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Json<ManagementHistoryJobResponse>, ApiError> {
    engine
        .get_management_service()
        .find_history_job_by_id(&job_id)
        .map(|job| history_job_to_management_history_job(&engine, job))
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("History job '{}' not found", job_id)))
}

async fn delete_history_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_management_service()
        .delete_history_job(&job_id)
        .map_err(job_mutation_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_history_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
    Json(request): Json<ManagementJobActionRequest>,
) -> Result<StatusCode, ApiError> {
    match required_action(&request)?.as_str() {
        "execute" => {
            engine
                .get_management_service()
                .execute_history_job(&job_id)
                .map_err(job_execute_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        action => Err(unsupported_job_action("history-jobs", action, &["execute"])),
    }
}

async fn list_suspended_jobs(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ManagementJobResponse>>, ApiError> {
    let query: ManagementListQuery = parse_query(&uri)?;
    list_management_jobs(
        &engine,
        &query,
        JobFamily::Suspended,
        suspended_job_to_management_job,
    )
}

async fn get_suspended_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Json<ManagementJobResponse>, ApiError> {
    engine
        .get_management_service()
        .find_suspended_job_by_id(&job_id)
        .map(|job| suspended_job_to_management_job(&engine, job))
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("Suspended job '{}' not found", job_id)))
}

async fn delete_suspended_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_management_service()
        .delete_suspended_job(&job_id)
        .map_err(job_mutation_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_suspended_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
    Json(request): Json<ManagementJobActionRequest>,
) -> Result<StatusCode, ApiError> {
    match required_action(&request)?.as_str() {
        "move" | "moveToExecutableJob" | "move-to-executable-job" | "retry" => {
            // Java `moveSuspendedJobToExecutableJob` semantics: the retry count
            // is preserved unchanged and activation is rejected while the
            // parent process instance is suspended.
            engine
                .get_management_service()
                .activate_suspended_job(&job_id)
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        "setRetries" | "set-retries" | "retries" => {
            let retries = request
                .retries
                .ok_or_else(|| ApiError::bad_request("Field 'retries' is required"))?;
            engine
                .get_management_service()
                .set_job_retries(&job_id, retries)
                .map_err(job_mutation_error)?;
            Ok(StatusCode::NO_CONTENT)
        }
        action => Err(unsupported_job_action(
            "suspended-jobs",
            action,
            &["move", "moveToExecutableJob", "retry", "setRetries"],
        )),
    }
}

async fn get_suspended_job_exception_stacktrace(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    let job = engine
        .get_management_service()
        .find_suspended_job_by_id(&job_id)
        .ok_or_else(|| ApiError::NotFound(format!("Suspended job '{}' not found", job_id)))?;
    job_stacktrace_response("Suspended job", job.timer_job_id, job.error_details)
}

fn jmx_runtime_response(
    state: &ManagementApiState,
    engine: &ProcessEngine,
    directory_state: &crate::DirectoryReadState,
) -> Result<JmxRuntimeResponse, ApiError> {
    ensure_management_api_enabled(&state.operations_support_contract)?;

    Ok(JmxRuntimeResponse {
        exposure: operations_exposure_name(state.operations_support_contract.exposure),
        management_api_enabled: state.operations_support_contract.management_api_enabled,
        directory_provider: directory_provider_name(state.directory_support_contract.provider),
        identity: merged_identity_counts(engine, directory_state)?,
        engine_name: engine.get_name().to_string(),
    })
}

fn jmx_connector_descriptor_response(
    state: &ManagementApiState,
    engine: &ProcessEngine,
) -> Result<JmxConnectorDescriptorResponse, ApiError> {
    ensure_native_compatible_connector_enabled(&state.operations_support_contract)?;

    Ok(JmxConnectorDescriptorResponse {
        connector_family: "native-http".to_string(),
        exposure: operations_exposure_name(state.operations_support_contract.exposure),
        management_api_enabled: state.operations_support_contract.management_api_enabled,
        transport: "http-json".to_string(),
        simulated_rmi_transport: false,
        endpoint_prefix: "/management/jmx".to_string(),
        engine_name: engine.get_name().to_string(),
        embedding: embedding_support_response(&state.runtime_embedding_contract),
        directory_provider: directory_provider_name(state.directory_support_contract.provider),
        enterprise_adapters: enterprise_adapter_names(&state.enterprise_support_contracts),
        object_family_breadth: object_family_breadth_name(
            state.operations_support_contract.object_family_breadth,
        ),
        support_statement: state
            .operations_support_contract
            .support_statement
            .to_string(),
    })
}

fn jmx_mbean_registry_response(
    state: &ManagementApiState,
    engine: &ProcessEngine,
) -> Result<JmxMbeanRegistryResponse, ApiError> {
    ensure_mbean_registry_enabled(&state.operations_support_contract)?;

    let mbeans = jmx_mbean_descriptors(&state.operations_support_contract);
    Ok(JmxMbeanRegistryResponse {
        domain: "org.flowable".to_string(),
        engine_name: engine.get_name().to_string(),
        exposure: operations_exposure_name(state.operations_support_contract.exposure),
        object_family_breadth: object_family_breadth_name(
            state.operations_support_contract.object_family_breadth,
        ),
        mbean_count: mbeans.len(),
        mbeans,
    })
}

fn jmx_operations_bus_response(
    state: &ManagementApiState,
    engine: &ProcessEngine,
    directory_state: &crate::DirectoryReadState,
) -> Result<JmxOperationsBusResponse, ApiError> {
    ensure_operations_bus_enabled(&state.operations_support_contract)?;

    let registry_mbean_count = jmx_mbean_descriptors(&state.operations_support_contract).len();
    Ok(JmxOperationsBusResponse {
        family: "bounded-native-jmx".to_string(),
        object_family_breadth: object_family_breadth_name(
            state.operations_support_contract.object_family_breadth,
        ),
        exposure: operations_exposure_name(state.operations_support_contract.exposure),
        management_api_enabled: state.operations_support_contract.management_api_enabled,
        engine_name: engine.get_name().to_string(),
        connector: JmxBusConnectorReference {
            path: primary_jmx_path("connector-descriptor"),
            connector_family: "native-http".to_string(),
            transport: "http-json".to_string(),
            simulated_rmi_transport: false,
        },
        registry: JmxBusRegistryReference {
            path: primary_jmx_path("mbean-registry"),
            domain: "org.flowable".to_string(),
            mbean_count: registry_mbean_count,
        },
        runtime_ledger: state
            .operations_support_contract
            .runtime_ledger_enabled
            .then(|| jmx_runtime_ledger_response(state, engine, directory_state))
            .transpose()?,
        timer_ledger: state
            .operations_support_contract
            .timer_ledger_enabled
            .then(|| jmx_timer_ledger_response(state, engine))
            .transpose()?,
        topology: state
            .operations_support_contract
            .topology_ledger_enabled
            .then(|| operations_topology_response(state, engine))
            .transpose()?,
    })
}

fn jmx_runtime_ledger_response(
    state: &ManagementApiState,
    engine: &ProcessEngine,
    directory_state: &crate::DirectoryReadState,
) -> Result<JmxRuntimeLedgerResponse, ApiError> {
    ensure_runtime_ledger_enabled(&state.operations_support_contract)?;

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_count = runtime_store.snapshot_process_instances(&mut session).len();
    let execution_count = runtime_store.snapshot_executions(&mut session).len();
    let task_count = engine
        .get_task_service()
        .create_task_query()
        .count()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?
        as usize;
    let variable_count = engine
        .get_variable_service()
        .create_variable_instance_query()
        .count()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?
        as usize;
    let event_subscription_count = engine
        .get_runtime_service()
        .create_event_subscription_query()
        .count()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?
        as usize;

    Ok(JmxRuntimeLedgerResponse {
        engine_name: engine.get_name().to_string(),
        exposure: operations_exposure_name(state.operations_support_contract.exposure),
        process_instance_count,
        execution_count,
        task_count,
        variable_count,
        event_subscription_count,
        identity: merged_identity_counts(engine, directory_state)?,
        directory_provider: directory_provider_name(state.directory_support_contract.provider),
    })
}

fn jmx_timer_ledger_response(
    state: &ManagementApiState,
    engine: &ProcessEngine,
) -> Result<JmxTimerLedgerResponse, ApiError> {
    ensure_timer_ledger_enabled(&state.operations_support_contract)?;

    let management_service = engine.get_management_service();
    let timer_jobs = management_service
        .create_timer_job_query()
        .list()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let active_timer_job_count = timer_jobs
        .iter()
        .filter(|job| job.lock_owner.is_none())
        .count();
    let locked_timer_job_count = timer_jobs.len() - active_timer_job_count;

    let runtime_service = engine.get_runtime_service();
    let coordinator = timer_coordinator_response(runtime_service.get_timer_coordinator_status());
    let nodes = runtime_service
        .list_timer_nodes()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?
        .into_iter()
        .map(timer_node_response)
        .collect();
    let metrics = timer_metrics_response(runtime_service.timer_metrics().as_ref());

    Ok(JmxTimerLedgerResponse {
        exposure: operations_exposure_name(state.operations_support_contract.exposure),
        management_api_enabled: state.operations_support_contract.management_api_enabled,
        total_timer_job_count: timer_jobs.len(),
        active_timer_job_count,
        locked_timer_job_count,
        coordinator,
        nodes,
        metrics,
    })
}

fn operations_topology_response(
    state: &ManagementApiState,
    engine: &ProcessEngine,
) -> Result<OperationsTopologyResponse, ApiError> {
    ensure_topology_ledger_enabled(&state.operations_support_contract)?;

    let runtime_service = engine.get_runtime_service();
    let nodes = runtime_service
        .list_timer_nodes()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?
        .into_iter()
        .map(timer_node_response)
        .collect();

    Ok(OperationsTopologyResponse {
        engine_name: engine.get_name().to_string(),
        embedding: embedding_support_response(&state.runtime_embedding_contract),
        enterprise: enterprise_support_response(
            &state.enterprise_support_contracts,
            &state.enterprise_support_statement,
        ),
        directory_provider: directory_provider_name(state.directory_support_contract.provider),
        directory_runtime_reads: DirectoryRuntimeReadsResponse {
            user: state.directory_support_contract.runtime_user_read_enabled,
            group: state.directory_support_contract.runtime_group_read_enabled,
            membership: state
                .directory_support_contract
                .runtime_membership_read_enabled,
        },
        operations: operations_support_response(&state.operations_support_contract),
        coordinator: timer_coordinator_response(runtime_service.get_timer_coordinator_status()),
        nodes,
    })
}

fn directory_support_response(contract: &DirectorySupportContract) -> DirectorySupportResponse {
    DirectorySupportResponse {
        provider: directory_provider_name(contract.provider),
        sync_on_bootstrap: contract.sync_on_bootstrap,
        transport: contract.transport.clone(),
        auth_mode: contract.auth_mode.clone(),
        deployment_mode: contract.deployment_mode.clone(),
        conflict_policy: contract.conflict_policy.clone(),
        filter_breadth: contract.filter_breadth.clone(),
        imported_user_count: contract.imported_user_count,
        imported_group_count: contract.imported_group_count,
        imported_membership_count: contract.imported_membership_count,
        runtime_user_read_enabled: contract.runtime_user_read_enabled,
        runtime_group_read_enabled: contract.runtime_group_read_enabled,
        runtime_membership_read_enabled: contract.runtime_membership_read_enabled,
        runtime_user_write_enabled: contract.runtime_user_write_enabled,
        runtime_group_write_enabled: contract.runtime_group_write_enabled,
        runtime_membership_write_enabled: contract.runtime_membership_write_enabled,
        runtime_reconcile_enabled: contract.runtime_reconcile_enabled,
        runtime_bidirectional_sync_enabled: contract.runtime_bidirectional_sync_enabled,
        support_statement: contract.support_statement.to_string(),
    }
}

fn directory_reconcile_response(
    state: &ManagementApiState,
    engine: &Arc<ProcessEngine>,
    directory_state: &crate::DirectoryReadState,
    apply: bool,
    mode: DirectoryReconcileMode,
) -> Result<DirectoryReconcileResponse, ApiError> {
    if !state.operations_support_contract.management_api_enabled {
        return Err(ApiError::NotFound(
            "Management API is disabled for the current operations contract".to_string(),
        ));
    }
    if !state.directory_support_contract.runtime_reconcile_enabled {
        return Err(ApiError::BadRequest(
            "Directory reconcile is not enabled for the current directory contract".to_string(),
        ));
    }

    let live_snapshot = directory_state.load_live_snapshot()?.ok_or_else(|| {
        ApiError::BadRequest(
            "Directory reconcile requires a live directory provider on the current target"
                .to_string(),
        )
    })?;

    let identity_service = engine.get_identity_service();
    let stored_users = identity_service
        .create_user_query()
        .list()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let stored_groups = identity_service
        .create_group_query()
        .list()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let stored_memberships = identity_service.list_memberships();

    let live_user_ids = live_snapshot
        .users
        .iter()
        .map(|user| user.id.clone())
        .collect::<BTreeSet<_>>();
    let live_group_ids = live_snapshot
        .groups
        .iter()
        .map(|group| group.id.clone())
        .collect::<BTreeSet<_>>();
    let live_membership_keys = live_snapshot
        .memberships
        .iter()
        .map(|membership| (membership.user_id.clone(), membership.group_id.clone()))
        .collect::<BTreeSet<_>>();

    let shadowed_user_ids = stored_users
        .iter()
        .filter(|user| live_user_ids.contains(&user.id))
        .map(|user| user.id.clone())
        .collect::<Vec<_>>();
    let shadowed_group_ids = stored_groups
        .iter()
        .filter(|group| live_group_ids.contains(&group.id))
        .map(|group| group.id.clone())
        .collect::<Vec<_>>();
    let shadowed_memberships = stored_memberships
        .iter()
        .filter(|membership| {
            live_membership_keys
                .contains(&(membership.user_id.clone(), membership.group_id.clone()))
                || live_user_ids.contains(&membership.user_id)
                || live_group_ids.contains(&membership.group_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    let owned_only_users = stored_users
        .iter()
        .filter(|user| !live_user_ids.contains(&user.id))
        .cloned()
        .collect::<Vec<_>>();
    let owned_only_groups = stored_groups
        .iter()
        .filter(|group| !live_group_ids.contains(&group.id))
        .cloned()
        .collect::<Vec<_>>();
    let owned_only_user_ids = owned_only_users
        .iter()
        .map(|user| user.id.clone())
        .collect::<Vec<_>>();
    let owned_only_group_ids = owned_only_groups
        .iter()
        .map(|group| group.id.clone())
        .collect::<Vec<_>>();
    let owned_only_user_id_set = owned_only_user_ids.iter().cloned().collect::<BTreeSet<_>>();
    let owned_only_group_id_set = owned_only_group_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let owned_only_memberships = stored_memberships
        .iter()
        .filter(|membership| {
            !live_membership_keys
                .contains(&(membership.user_id.clone(), membership.group_id.clone()))
                && (live_user_ids.contains(&membership.user_id)
                    || owned_only_user_id_set.contains(&membership.user_id))
                && (live_group_ids.contains(&membership.group_id)
                    || owned_only_group_id_set.contains(&membership.group_id))
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut added_users = 0;
    let mut added_groups = 0;
    let mut added_memberships = 0;
    let mut removed_users = 0;
    let mut removed_groups = 0;
    let mut removed_memberships = 0;

    if apply {
        let mut session = engine.get_runtime_store().create_session().unwrap();
        match mode {
            DirectoryReconcileMode::LiveWins => {
                for membership in &shadowed_memberships {
                    identity_service.delete_membership_in_session(
                        &membership.user_id,
                        &membership.group_id,
                        &mut session,
                    );
                }
                removed_memberships = shadowed_memberships.len();
                for user_id in &shadowed_user_ids {
                    identity_service.delete_user_in_session(user_id, &mut session);
                }
                removed_users = shadowed_user_ids.len();
                for group_id in &shadowed_group_ids {
                    identity_service.delete_group_in_session(group_id, &mut session);
                }
                removed_groups = shadowed_group_ids.len();
            }
            DirectoryReconcileMode::OwnedToLive => {
                for user in &owned_only_users {
                    directory_state.save_live_user(user.clone())?.ok_or_else(|| {
                        ApiError::BadRequest(
                            "Directory reconcile requires a live directory provider on the current target"
                                .to_string(),
                        )
                    })?;
                }
                added_users = owned_only_users.len();
                for group in &owned_only_groups {
                    directory_state.save_live_group(group.clone())?.ok_or_else(|| {
                        ApiError::BadRequest(
                            "Directory reconcile requires a live directory provider on the current target"
                                .to_string(),
                        )
                    })?;
                }
                added_groups = owned_only_groups.len();
                for membership in &owned_only_memberships {
                    directory_state
                        .create_live_membership(&membership.user_id, &membership.group_id)?
                        .ok_or_else(|| {
                            ApiError::BadRequest(
                                "Directory reconcile requires a live directory provider on the current target"
                                    .to_string(),
                            )
                        })?;
                }
                added_memberships = owned_only_memberships.len();

                for membership in &owned_only_memberships {
                    identity_service.delete_membership_in_session(
                        &membership.user_id,
                        &membership.group_id,
                        &mut session,
                    );
                }
                removed_memberships = owned_only_memberships.len();
                for user_id in &owned_only_user_ids {
                    identity_service.delete_user_in_session(user_id, &mut session);
                }
                removed_users = owned_only_user_ids.len();
                for group_id in &owned_only_group_ids {
                    identity_service.delete_group_in_session(group_id, &mut session);
                }
                removed_groups = owned_only_group_ids.len();
            }
        }
        session.flush_and_commit().unwrap();
    }

    Ok(DirectoryReconcileResponse {
        provider: directory_provider_name(state.directory_support_contract.provider),
        mode: directory_reconcile_mode_name(mode),
        supported: true,
        applied: apply,
        live_user_count: live_snapshot.users.len() + added_users,
        live_group_count: live_snapshot.groups.len() + added_groups,
        live_membership_count: live_snapshot.memberships.len() + added_memberships,
        shadowed_user_ids,
        shadowed_group_ids,
        shadowed_memberships: shadowed_memberships
            .into_iter()
            .map(|membership| MembershipKeyResponse {
                user_id: membership.user_id,
                group_id: membership.group_id,
            })
            .collect(),
        owned_only_user_ids,
        owned_only_group_ids,
        owned_only_memberships: owned_only_memberships
            .into_iter()
            .map(|membership| MembershipKeyResponse {
                user_id: membership.user_id,
                group_id: membership.group_id,
            })
            .collect(),
        added_users,
        added_groups,
        added_memberships,
        removed_users,
        removed_groups,
        removed_memberships,
    })
}

fn parse_directory_reconcile_mode(value: Option<&str>) -> Result<DirectoryReconcileMode, ApiError> {
    match value
        .unwrap_or("live-wins")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "live-wins" => Ok(DirectoryReconcileMode::LiveWins),
        "owned-to-live" => Ok(DirectoryReconcileMode::OwnedToLive),
        other => Err(ApiError::BadRequest(format!(
            "Unsupported directory reconcile mode '{}'",
            other
        ))),
    }
}

fn directory_reconcile_mode_name(mode: DirectoryReconcileMode) -> String {
    match mode {
        DirectoryReconcileMode::LiveWins => "live-wins".to_string(),
        DirectoryReconcileMode::OwnedToLive => "owned-to-live".to_string(),
    }
}

fn operations_support_response(contract: &OperationsSupportContract) -> OperationsSupportResponse {
    OperationsSupportResponse {
        exposure: operations_exposure_name(contract.exposure),
        management_api_enabled: contract.management_api_enabled,
        runtime_ledger_enabled: contract.runtime_ledger_enabled,
        timer_ledger_enabled: contract.timer_ledger_enabled,
        topology_ledger_enabled: contract.topology_ledger_enabled,
        native_compatible_connector_enabled: contract.native_compatible_connector_enabled,
        mbean_registry_enabled: contract.mbean_registry_enabled,
        operations_bus_enabled: contract.operations_bus_enabled,
        object_family_breadth: object_family_breadth_name(contract.object_family_breadth),
        support_statement: contract.support_statement.to_string(),
    }
}

fn topology_certification_response(
    contract: &TopologyCertificationContract,
) -> TopologyCertificationResponse {
    TopologyCertificationResponse {
        profile: topology_profile_name(contract.profile),
        ingress: contract.ingress.clone(),
        packaging: contract.packaging.clone(),
        startup_certified: contract.startup_certified,
        auth_certified: contract.auth_certified,
        cutover_certified: contract.cutover_certified,
        rollback_certified: contract.rollback_certified,
        recovery_certified: contract.recovery_certified,
        supported_historical_ingress: contract.supported_historical_ingress.clone(),
        support_statement: contract.support_statement.to_string(),
    }
}

fn embedding_support_response(contract: &RuntimeEmbeddingContract) -> EmbeddingSupportResponse {
    EmbeddingSupportResponse {
        mode: runtime_embedding_mode_name(contract.mode),
        profile: runtime_embedding_profile_name(contract.profile),
    }
}

fn enterprise_support_response(
    contracts: &[EnterpriseAdapterSupportContract],
    support_statement: &str,
) -> EnterpriseSupportResponse {
    EnterpriseSupportResponse {
        adapter_count: contracts.len(),
        adapters: enterprise_adapter_names(contracts),
        support_kinds: contracts
            .iter()
            .map(|contract| enterprise_support_kind_name(contract.support_kind))
            .collect(),
        support_statement: support_statement.to_string(),
    }
}

fn enterprise_adapter_names(contracts: &[EnterpriseAdapterSupportContract]) -> Vec<String> {
    contracts
        .iter()
        .map(|contract| enterprise_adapter_family_name(contract.family))
        .collect()
}

fn jmx_mbean_descriptors(contract: &OperationsSupportContract) -> Vec<JmxMbeanDescriptorResponse> {
    let mut mbeans = Vec::new();

    if contract.native_compatible_connector_enabled {
        mbeans.push(JmxMbeanDescriptorResponse {
            object_name: "org.flowable.management:type=Connector,name=NativeCompatible".to_string(),
            kind: "connector-descriptor".to_string(),
            path: primary_jmx_path("connector-descriptor"),
            attributes: vec![
                "connectorFamily".to_string(),
                "transport".to_string(),
                "engineName".to_string(),
                "directoryProvider".to_string(),
                "objectFamilyBreadth".to_string(),
            ],
            operations: Vec::new(),
        });
    }

    if contract.runtime_ledger_enabled {
        mbeans.push(JmxMbeanDescriptorResponse {
            object_name: "org.flowable.management:type=RuntimeLedger,name=ProcessEngine"
                .to_string(),
            kind: "runtime-ledger".to_string(),
            path: primary_jmx_path("runtime-ledger"),
            attributes: vec![
                "processInstanceCount".to_string(),
                "executionCount".to_string(),
                "taskCount".to_string(),
                "variableCount".to_string(),
                "eventSubscriptionCount".to_string(),
            ],
            operations: Vec::new(),
        });
    }

    if contract.timer_ledger_enabled {
        mbeans.push(JmxMbeanDescriptorResponse {
            object_name: "org.flowable.management:type=TimerLedger,name=Coordinator".to_string(),
            kind: "timer-ledger".to_string(),
            path: primary_jmx_path("timer-ledger"),
            attributes: vec![
                "totalTimerJobCount".to_string(),
                "activeTimerJobCount".to_string(),
                "lockedTimerJobCount".to_string(),
                "coordinator".to_string(),
                "nodes".to_string(),
            ],
            operations: Vec::new(),
        });
    }

    if contract.topology_ledger_enabled {
        mbeans.push(JmxMbeanDescriptorResponse {
            object_name: "org.flowable.management:type=TopologyLedger,name=Operations".to_string(),
            kind: "operations-topology".to_string(),
            path: primary_operations_path("topology"),
            attributes: vec![
                "embedding".to_string(),
                "enterprise".to_string(),
                "directoryProvider".to_string(),
                "operations".to_string(),
                "nodes".to_string(),
            ],
            operations: Vec::new(),
        });
    }

    if contract.operations_bus_enabled {
        mbeans.push(JmxMbeanDescriptorResponse {
            object_name: "org.flowable.management:type=OperationsBus,name=NativeCompatible"
                .to_string(),
            kind: "operations-bus".to_string(),
            path: primary_jmx_path("operations-bus"),
            attributes: vec![
                "family".to_string(),
                "connector".to_string(),
                "registry".to_string(),
                "runtimeLedger".to_string(),
                "timerLedger".to_string(),
                "topology".to_string(),
            ],
            operations: Vec::new(),
        });
    }

    mbeans
}

fn ensure_management_api_enabled(contract: &OperationsSupportContract) -> Result<(), ApiError> {
    if contract.management_api_enabled {
        Ok(())
    } else {
        Err(ApiError::NotFound(
            "Management API is disabled for the current operations contract".to_string(),
        ))
    }
}

fn ensure_runtime_ledger_enabled(contract: &OperationsSupportContract) -> Result<(), ApiError> {
    ensure_management_api_enabled(contract)?;
    if contract.runtime_ledger_enabled {
        Ok(())
    } else {
        Err(ApiError::NotFound(
            "Runtime ledger is disabled for the current operations contract".to_string(),
        ))
    }
}

fn ensure_timer_ledger_enabled(contract: &OperationsSupportContract) -> Result<(), ApiError> {
    ensure_management_api_enabled(contract)?;
    if contract.timer_ledger_enabled {
        Ok(())
    } else {
        Err(ApiError::NotFound(
            "Timer ledger is disabled for the current operations contract".to_string(),
        ))
    }
}

fn ensure_topology_ledger_enabled(contract: &OperationsSupportContract) -> Result<(), ApiError> {
    ensure_management_api_enabled(contract)?;
    if contract.topology_ledger_enabled {
        Ok(())
    } else {
        Err(ApiError::NotFound(
            "Operations topology ledger is disabled for the current operations contract"
                .to_string(),
        ))
    }
}

fn ensure_native_compatible_connector_enabled(
    contract: &OperationsSupportContract,
) -> Result<(), ApiError> {
    ensure_management_api_enabled(contract)?;
    if contract.native_compatible_connector_enabled {
        Ok(())
    } else {
        Err(ApiError::NotFound(
            "Native-compatible JMX connector is disabled for the current operations contract"
                .to_string(),
        ))
    }
}

fn ensure_mbean_registry_enabled(contract: &OperationsSupportContract) -> Result<(), ApiError> {
    ensure_management_api_enabled(contract)?;
    if contract.mbean_registry_enabled {
        Ok(())
    } else {
        Err(ApiError::NotFound(
            "Native-compatible JMX MBean registry is disabled for the current operations contract"
                .to_string(),
        ))
    }
}

fn ensure_operations_bus_enabled(contract: &OperationsSupportContract) -> Result<(), ApiError> {
    ensure_management_api_enabled(contract)?;
    if contract.operations_bus_enabled {
        Ok(())
    } else {
        Err(ApiError::NotFound(
            "Native-compatible JMX operations bus is disabled for the current operations contract"
                .to_string(),
        ))
    }
}

fn primary_jmx_path(suffix: &str) -> String {
    format!("/management/jmx/{suffix}")
}

fn primary_operations_path(suffix: &str) -> String {
    format!("/management/operations/{suffix}")
}

fn merged_identity_counts(
    engine: &ProcessEngine,
    directory_state: &crate::DirectoryReadState,
) -> Result<IdentityCounts, ApiError> {
    let identity_service = engine.get_identity_service();
    let users = identity_service
        .create_user_query()
        .list()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let groups = identity_service
        .create_group_query()
        .list()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let tokens = identity_service
        .create_token_query()
        .list()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let live_snapshot = directory_state.load_live_snapshot()?;

    let mut user_ids = users
        .iter()
        .map(|user| user.id.clone())
        .collect::<BTreeSet<_>>();
    let mut group_ids = groups
        .iter()
        .map(|group| group.id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(snapshot) = live_snapshot {
        user_ids.extend(snapshot.users.into_iter().map(|user| user.id));
        group_ids.extend(snapshot.groups.into_iter().map(|group| group.id));
    }

    Ok(IdentityCounts {
        users: user_ids.len(),
        groups: group_ids.len(),
        tokens: tokens.len(),
    })
}

fn engine_properties(
    engine: &ProcessEngine,
) -> Result<BTreeMap<String, EnginePropertyResponse>, ApiError> {
    let mut properties = BTreeMap::from([
        (
            "engineName".to_string(),
            EnginePropertyResponse {
                name: "engineName".to_string(),
                value: engine.get_name().to_string(),
                revision: 1,
            },
        ),
        (
            "version".to_string(),
            EnginePropertyResponse {
                name: "version".to_string(),
                value: engine.get_version().to_string(),
                revision: 1,
            },
        ),
        (
            "schemaTableCount".to_string(),
            EnginePropertyResponse {
                name: "schemaTableCount".to_string(),
                value: table_names(engine)?.len().to_string(),
                revision: 1,
            },
        ),
    ]);

    for property in stored_engine_properties(engine)? {
        properties.insert(
            property.name.clone(),
            EnginePropertyResponse {
                name: property.name,
                value: property.value,
                revision: property.revision,
            },
        );
    }
    Ok(properties)
}

fn stored_engine_properties(
    engine: &ProcessEngine,
) -> Result<Vec<flowable_engine::persistence::runtime_store::EngineProperty>, ApiError> {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;
    runtime_store
        .list_engine_properties(&mut session)
        .map_err(|e| ApiError::InternalServerError(e.to_string()))
}

fn required_property_name(name: Option<&str>) -> Result<String, ApiError> {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| ApiError::bad_request("Field 'name' is required"))
}

fn table_names(engine: &ProcessEngine) -> Result<Vec<String>, ApiError> {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let rows = session
        .raw_query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            DbParams::new(),
        )
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let mut names = Vec::new();
    for row in rows {
        if let Some(name) = row.get_text("name") {
            names.push(name);
        }
    }
    Ok(names)
}

fn table_response(name: String, count: usize) -> TableResponse {
    TableResponse {
        url: format!("/management/tables/{name}"),
        name,
        count,
    }
}

fn ensure_table_exists(engine: &ProcessEngine, table_name: &str) -> Result<(), ApiError> {
    if !is_safe_table_name(table_name) {
        return Err(ApiError::bad_request(format!(
            "Invalid table name '{}'",
            table_name
        )));
    }
    if table_names(engine)?.iter().any(|name| name == table_name) {
        Ok(())
    } else {
        Err(ApiError::NotFound(format!(
            "Table '{}' not found",
            table_name
        )))
    }
}

fn count_table_rows(engine: &ProcessEngine, table_name: &str) -> Result<usize, ApiError> {
    ensure_table_identifier(table_name)?;
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let sql = format!("SELECT COUNT(*) AS RES_ FROM {table_name}");
    let count = session
        .raw_query_one(&sql, DbParams::new())
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?
        .and_then(|r| r.get_integer("RES_"))
        .unwrap_or(0);
    Ok(count as usize)
}

fn table_columns(
    engine: &ProcessEngine,
    table_name: &str,
) -> Result<Vec<TableColumnResponse>, ApiError> {
    ensure_table_identifier(table_name)?;
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let columns = session
        .table_columns(table_name)
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    Ok(columns
        .into_iter()
        .map(|column| TableColumnResponse {
            name: column.name,
            column_type: column.data_type,
            nullable: column.nullable,
            primary_key: column.primary_key,
        })
        .collect())
}

fn db_value_to_json(v: &DbValue) -> Value {
    match v {
        DbValue::Null | DbValue::NullInteger | DbValue::NullBoolean | DbValue::NullBlob => {
            Value::Null
        }
        DbValue::Text(s) => Value::String(s.clone()),
        DbValue::Integer(i) => Value::Number(serde_json::Number::from(*i)),
        DbValue::Real(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        DbValue::Boolean(b) => Value::Bool(*b),
        DbValue::Blob(bytes) => Value::String(
            bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>(),
        ),
    }
}

fn table_data(
    engine: &ProcessEngine,
    table_name: &str,
    sort: Option<&TableDataSort>,
) -> Result<Vec<Value>, ApiError> {
    ensure_table_identifier(table_name)?;
    let columns = table_columns(engine, table_name)?;
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let sql = if let Some(sort) = sort {
        format!(
            "SELECT * FROM {table_name} ORDER BY {} {}",
            sort.column, sort.sql_direction
        )
    } else {
        format!("SELECT * FROM {table_name}")
    };
    let rows = session
        .raw_query(&sql, DbParams::new())
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let column_names = columns
        .into_iter()
        .map(|column| column.name)
        .collect::<Vec<_>>();
    let mut data = Vec::new();
    for row in rows {
        let mut object = Map::new();
        for column_name in &column_names {
            let value = row
                .get(column_name)
                .map(db_value_to_json)
                .unwrap_or(Value::Null);
            object.insert(column_name.clone(), value);
        }
        data.push(Value::Object(object));
    }
    Ok(data)
}

struct TableDataSort {
    column: String,
    order: String,
    sql_direction: &'static str,
}

fn table_data_sort(
    engine: &ProcessEngine,
    table_name: &str,
    query: &TableDataQuery,
) -> Result<Option<TableDataSort>, ApiError> {
    match (
        query.order_ascending_column.as_deref(),
        query.order_descending_column.as_deref(),
    ) {
        (Some(_), Some(_)) => Err(ApiError::bad_request(
            "Only one of 'orderAscendingColumn' or 'orderDescendingColumn' can be supplied.",
        )),
        (Some(column), None) => {
            table_data_sort_for_column(engine, table_name, column, "asc", "ASC")
        }
        (None, Some(column)) => {
            table_data_sort_for_column(engine, table_name, column, "desc", "DESC")
        }
        (None, None) => Ok(None),
    }
}

fn table_data_sort_for_column(
    engine: &ProcessEngine,
    table_name: &str,
    column: &str,
    order: &str,
    sql_direction: &'static str,
) -> Result<Option<TableDataSort>, ApiError> {
    ensure_table_identifier(column)?;
    if table_columns(engine, table_name)?
        .iter()
        .any(|table_column| table_column.name == column)
    {
        Ok(Some(TableDataSort {
            column: column.to_string(),
            order: order.to_string(),
            sql_direction,
        }))
    } else {
        Err(ApiError::bad_request(format!(
            "Column '{}' does not exist in table '{}'",
            column, table_name
        )))
    }
}

fn paginate_table_data(
    rows: Vec<Value>,
    query: &TableDataQuery,
    sort: Option<TableDataSort>,
) -> TableDataResponse {
    let total = rows.len();
    let start = query.start.min(total);
    let size = query.size.unwrap_or(10);
    let data = rows.into_iter().skip(start).take(size).collect::<Vec<_>>();
    let returned_size = data.len();
    let (sort, order) = sort
        .map(|sort| (Some(sort.column), Some(sort.order)))
        .unwrap_or((None, None));

    TableDataResponse {
        start,
        size: returned_size,
        total,
        sort,
        order,
        data,
    }
}

fn ensure_table_identifier(table_name: &str) -> Result<(), ApiError> {
    if is_safe_table_name(table_name) {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "Invalid table name '{}'",
            table_name
        )))
    }
}

fn is_safe_table_name(table_name: &str) -> bool {
    !table_name.is_empty()
        && table_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn timer_job_to_management_job(
    engine: &ProcessEngine,
    job: RuntimeTimerJobState,
) -> ManagementJobResponse {
    timer_job_to_management_job_with_type(engine, job, "timer", "/management/timer-jobs")
}

fn executable_job_to_management_job(
    engine: &ProcessEngine,
    job: RuntimeTimerJobState,
) -> ManagementJobResponse {
    timer_job_to_management_job_with_type(engine, job, "executable", "/management/jobs")
}

fn deadletter_job_to_management_job(
    engine: &ProcessEngine,
    job: RuntimeTimerJobState,
) -> ManagementJobResponse {
    timer_job_to_management_job_with_type(engine, job, "deadletter", "/management/deadletter-jobs")
}

fn history_job_to_management_history_job(
    engine: &ProcessEngine,
    job: RuntimeTimerJobState,
) -> ManagementHistoryJobResponse {
    let management_service = engine.get_management_service();
    let tenant_id = job
        .tenant_id
        .clone()
        .or_else(|| management_service.job_tenant_id(&job));
    let job_handler_type = job_handler_type(&job, JobFamily::History.default_job_type());
    // Java GET fills advancedJobHandlerConfiguration via getHistoryJobHistoryJson
    // (advanced config byte array). Fall back to the inline time_duration payload
    // when advanced config was not separately persisted.
    let advanced = job
        .advanced_job_handler_configuration
        .clone()
        .or_else(|| job.time_duration.clone());
    let id = job.timer_job_id;
    ManagementHistoryJobResponse {
        url: format!("/management/history-jobs/{id}"),
        id,
        scope_type: job.scope_type,
        retries: job.retries.unwrap_or(1),
        exception_message: job.error_message,
        job_handler_type,
        job_handler_configuration: job.job_handler_configuration,
        advanced_job_handler_configuration: advanced,
        tenant_id,
        custom_values: job.custom_values,
        create_time: format_millis(job.create_time),
        lock_owner: job.lock_owner,
        lock_expiration_time: format_millis(job.lock_expiration_time),
    }
}

fn suspended_job_to_management_job(
    engine: &ProcessEngine,
    job: RuntimeTimerJobState,
) -> ManagementJobResponse {
    timer_job_to_management_job_with_type(engine, job, "suspended", "/management/suspended-jobs")
}

fn timer_job_to_management_job_with_type(
    engine: &ProcessEngine,
    job: RuntimeTimerJobState,
    job_type: &str,
    url_prefix: &str,
) -> ManagementJobResponse {
    let management_service = engine.get_management_service();
    // Prefer denormalized columns; fall back to execution joins for legacy rows.
    let process_definition_id = job
        .process_definition_id
        .clone()
        .or_else(|| management_service.job_process_definition_id(&job));
    let tenant_id = job
        .tenant_id
        .clone()
        .or_else(|| management_service.job_tenant_id(&job));
    let element_name = job
        .element_name
        .clone()
        .or_else(|| management_service.job_element_name(&job));
    let handler_type = job_handler_type(&job, job_type);
    let process_instance_id = non_empty_string(job.process_instance_id.clone());
    let execution_id = non_empty_string(job.execution_id.clone());
    let element_id = non_empty_string(job.activity_id.clone());
    let process_instance_url = process_instance_id
        .as_ref()
        .map(|id| format!("/runtime/process-instances/{id}"));
    let process_definition_url = process_definition_id
        .as_ref()
        .map(|id| format!("/repository/process-definitions/{id}"));
    let execution_url = execution_id
        .as_ref()
        .map(|id| format!("/runtime/executions/{id}"));
    let id = job.timer_job_id;
    ManagementJobResponse {
        url: format!("{url_prefix}/{id}"),
        id,
        job_type: job_type.to_string(),
        correlation_id: job.correlation_id,
        process_instance_id,
        process_instance_url,
        process_definition_id,
        process_definition_url,
        execution_id,
        execution_url,
        element_id,
        element_name,
        handler_type,
        lock_owner: job.lock_owner,
        due_date: format_millis(job.due_time),
        create_time: format_millis(job.create_time),
        lock_expiration_time: format_millis(job.lock_expiration_time),
        retries: job.retries.unwrap_or(1),
        exception_message: job.error_message,
        tenant_id,
    }
}

fn non_empty_string(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn format_millis(value: Option<i64>) -> Option<String> {
    value
        .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
        .map(|date| date.to_rfc3339())
}

fn job_handler_type(job: &RuntimeTimerJobState, job_type: &str) -> Option<String> {
    if let Some(handler_type) = job
        .handler_type
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return Some(handler_type.to_string());
    }
    // Legacy rows without stored handler_type fall back to job_state synthesis.
    let handler_type = match job.job_state.as_deref() {
        Some("async") => "async",
        Some("history") => "history",
        Some("suspended") => "suspended",
        Some("deadletter") => "deadletter",
        Some("timer") | None => "timer",
        Some("executable") => "timer",
        Some(other) => other,
    };
    if handler_type.is_empty() {
        Some(job_type.to_string())
    } else {
        Some(handler_type.to_string())
    }
}

fn job_stacktrace_response(
    job_kind: &str,
    job_id: String,
    error_details: Option<String>,
) -> Result<Response, ApiError> {
    error_details
        .filter(|details| !details.is_empty())
        .map(|details| ([(header::CONTENT_TYPE, "text/plain")], details).into_response())
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "{} with id '{}' does not have an exception stacktrace",
                job_kind, job_id
            ))
        })
}

fn required_action(request: &ManagementJobActionRequest) -> Result<String, ApiError> {
    request
        .action
        .as_ref()
        .filter(|action| !action.trim().is_empty())
        .map(|action| action.trim().to_string())
        .ok_or_else(|| ApiError::bad_request("Field 'action' is required"))
}

fn job_family_to_runtime(family: JobFamily) -> RuntimeJobFamily {
    match family {
        JobFamily::Executable => RuntimeJobFamily::Executable,
        JobFamily::Timer => RuntimeJobFamily::Timer,
        JobFamily::Deadletter => RuntimeJobFamily::Deadletter,
        JobFamily::Suspended => RuntimeJobFamily::Suspended,
        JobFamily::History => RuntimeJobFamily::History,
    }
}

/// List one management job family via the direct engine query so filters,
/// totals, and paging share one code path (ADR-3 / P65-job-query).
fn list_management_jobs<T>(
    engine: &ProcessEngine,
    query: &ManagementListQuery,
    family: JobFamily,
    map: impl Fn(&ProcessEngine, RuntimeTimerJobState) -> T,
) -> Result<Json<PagedResponse<T>>, ApiError> {
    let page = query_management_jobs(engine, query, family)?;
    let data = page
        .data
        .into_iter()
        .map(|job| map(engine, job))
        .collect::<Vec<_>>();
    Ok(Json(PagedResponse {
        data,
        total: page.total,
        start: page.start,
        size: page.size,
        sort: Some(page.sort),
        order: Some(page.order),
    }))
}

fn query_management_jobs(
    engine: &ProcessEngine,
    query: &ManagementListQuery,
    family: JobFamily,
) -> Result<flowable_engine::engine::management_service::RuntimeJobQueryResult, ApiError> {
    // Java rejects the combination as soon as both parameter names are
    // present, regardless of their values (JobCollectionResource).
    if family != JobFamily::History && query.timers_only.is_some() && query.messages_only.is_some()
    {
        return Err(ApiError::bad_request(
            "Only one of 'timersOnly' or 'messagesOnly' can be supplied.",
        ));
    }
    if query.external_workers.unwrap_or(false)
        && (query.timers_only.unwrap_or(false) || query.messages_only.unwrap_or(false))
    {
        return Err(ApiError::bad_request(
            "Only one of 'timersOnly', 'messagesOnly', or 'externalWorkers' can be supplied.",
        ));
    }

    let sort = query.sort.clone().unwrap_or_else(|| "id".to_string());
    if !family.allowed_sorts().contains(&sort.as_str()) {
        return Err(ApiError::bad_request(format!(
            "Unsupported sort property '{sort}' for BPMN management jobs. Supported sort properties: {}",
            family.allowed_sorts().join(", ")
        )));
    }
    match query.order.as_deref() {
        None | Some("asc") | Some("desc") => {}
        Some(order) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported order '{order}' for BPMN management jobs. Supported orders: asc, desc"
            )));
        }
    }

    let due_before = query
        .due_before
        .as_deref()
        .map(|value| parse_job_date_millis("dueBefore", value))
        .transpose()?;
    let due_after = query
        .due_after
        .as_deref()
        .map(|value| parse_job_date_millis("dueAfter", value))
        .transpose()?;
    let handler_types = query
        .handler_types
        .as_deref()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let runtime_family = family != JobFamily::History;
    let page_size = query.size.unwrap_or(MANAGEMENT_JOB_DEFAULT_PAGE_SIZE);

    let mut runtime_query = engine
        .get_management_service()
        .create_runtime_job_query()
        .family(job_family_to_runtime(family))
        .order_by(sort)
        .page(query.start, page_size);
    runtime_query = if is_desc_order(query.order.as_deref()) {
        runtime_query.desc()
    } else {
        runtime_query.asc()
    };

    if let Some(id) = query.id.as_deref() {
        runtime_query = runtime_query.id(id);
    }
    if let Some(process_instance_id) = query.process_instance_id.as_deref() {
        runtime_query = runtime_query.process_instance_id(process_instance_id);
    }
    if runtime_family && query.without_process_instance_id.unwrap_or(false) {
        runtime_query = runtime_query.without_process_instance_id();
    }
    if let Some(process_definition_id) = query.process_definition_id.as_deref() {
        runtime_query = runtime_query.process_definition_id(process_definition_id);
    }
    if let Some(execution_id) = query.execution_id.as_deref() {
        runtime_query = runtime_query.execution_id(execution_id);
    }
    if runtime_family {
        if let Some(element_id) = query.element_id.as_deref() {
            runtime_query = runtime_query.element_id(element_id);
        }
        if let Some(element_name) = query.element_name.as_deref() {
            runtime_query = runtime_query.element_name(element_name);
        }
        // Real withoutScope predicates (P65): SCOPE_ID_/SCOPE_TYPE_ null/empty.
        if query.without_scope_id.unwrap_or(false) {
            runtime_query = runtime_query.without_scope_id();
        }
        if query.without_scope_type.unwrap_or(false) {
            runtime_query = runtime_query.without_scope_type();
        }
        if let Some(scope_id) = query.scope_id.as_deref() {
            runtime_query = runtime_query.scope_id(scope_id);
        }
        if let Some(sub_scope_id) = query.sub_scope_id.as_deref() {
            runtime_query = runtime_query.sub_scope_id(sub_scope_id);
        }
        if let Some(scope_type) = query.scope_type.as_deref() {
            runtime_query = runtime_query.scope_type(scope_type);
        }
        if let Some(scope_definition_id) = query.scope_definition_id.as_deref() {
            runtime_query = runtime_query.scope_definition_id(scope_definition_id);
        }
        if let Some(case_definition_key) = query.case_definition_key.as_deref() {
            runtime_query = runtime_query.case_definition_key(case_definition_key);
        }
        if let Some(category) = query.category.as_deref() {
            runtime_query = runtime_query.category(category);
        }
        if let Some(category_like) = query.category_like.as_deref() {
            runtime_query = runtime_query.category_like(category_like);
        }
        if let Some(correlation_id) = query.correlation_id.as_deref() {
            runtime_query = runtime_query.correlation_id(correlation_id);
        }
        if query.external_workers.unwrap_or(false) {
            runtime_query = runtime_query.external_workers();
        }
    }
    if let Some(handler_type) = query.handler_type.as_deref() {
        runtime_query = runtime_query.handler_type(handler_type);
    }
    if !handler_types.is_empty() {
        runtime_query = runtime_query.handler_types(handler_types);
    }
    if query.with_retries_left.unwrap_or(false) {
        runtime_query = runtime_query.with_retries_left();
    }
    if family == JobFamily::Suspended && query.no_retries_left.unwrap_or(false) {
        runtime_query = runtime_query.no_retries_left();
    }
    if query.executable.unwrap_or(false) {
        runtime_query = runtime_query.executable();
    }
    if query.timers_only.unwrap_or(false) {
        runtime_query = runtime_query.timers_only();
    }
    if query.messages_only.unwrap_or(false) {
        runtime_query = runtime_query.messages_only();
    }
    if let Some(due_before) = due_before {
        runtime_query = runtime_query.due_before(due_before);
    }
    if let Some(due_after) = due_after {
        runtime_query = runtime_query.due_after(due_after);
    }
    if query.with_exception.unwrap_or(false) {
        runtime_query = runtime_query.with_exception();
    }
    if query.without_exception.unwrap_or(false) {
        runtime_query = runtime_query.without_exception();
    }
    if let Some(exception_message) = query.exception_message.as_deref() {
        runtime_query = runtime_query.exception_message(exception_message);
    }
    if query.locked.unwrap_or(false) {
        runtime_query = runtime_query.locked();
    }
    if query.unlocked.unwrap_or(false) {
        runtime_query = runtime_query.unlocked();
    }
    if let Some(tenant_id) = query.tenant_id.as_deref() {
        runtime_query = runtime_query.tenant_id(tenant_id);
    }
    if let Some(tenant_id_like) = query.tenant_id_like.as_deref() {
        runtime_query = runtime_query.tenant_id_like(tenant_id_like);
    }
    if query.without_tenant_id.unwrap_or(false) {
        runtime_query = runtime_query.without_tenant_id();
    }

    runtime_query
        .list_page()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))
}

fn is_desc_order(order: Option<&str>) -> bool {
    matches!(order, Some("desc"))
}

fn parse_job_date_millis(field: &str, value: &str) -> Result<i64, ApiError> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.timestamp_millis())
        .map_err(|_| ApiError::bad_request(format!("Invalid date-time value for '{field}'")))
}

fn unsupported_job_action(family: &str, action: &str, supported: &[&str]) -> ApiError {
    if supported.is_empty() {
        ApiError::bad_request(format!(
            "Unsupported action '{}' for management/{}. No POST actions are currently supported for this job family",
            action, family
        ))
    } else {
        ApiError::bad_request(format!(
            "Unsupported action '{}' for management/{}. Supported actions: {}",
            action,
            family,
            supported.join(", ")
        ))
    }
}

fn job_mutation_error(error: flowable_engine::error::FlowableError) -> ApiError {
    match error {
        flowable_engine::error::FlowableError::NotFound(message) => ApiError::NotFound(message),
        flowable_engine::error::FlowableError::ExecutionError(message)
        | flowable_engine::error::FlowableError::DeploymentValidationError(message)
        | flowable_engine::error::FlowableError::UnsupportedElement {
            element_type: message,
            ..
        } => ApiError::BadRequest(message),
        other => ApiError::InternalServerError(other.to_string()),
    }
}

/// Java propagates job *execution* failures as 500 (JobResource executeJob
/// docs); only an unknown job id stays a 404. Validation failures (unsupported
/// action) never reach this mapper and remain 400.
fn job_execute_error(error: flowable_engine::error::FlowableError) -> ApiError {
    match error {
        flowable_engine::error::FlowableError::NotFound(message) => ApiError::NotFound(message),
        other => ApiError::InternalServerError(other.to_string()),
    }
}

fn timer_coordinator_response(
    status: flowable_engine::persistence::runtime_store::TimerCoordinatorStatus,
) -> TimerCoordinatorResponse {
    TimerCoordinatorResponse {
        leader_node_id: status.leader_node_id,
        fencing_token: status.fencing_token,
        lease_expiry_time: status.lease_expiry_time,
        status: match status.status {
            flowable_engine::persistence::runtime_store::CoordinatorLeadershipStatus::NoLeader => {
                "no-leader".to_string()
            }
            flowable_engine::persistence::runtime_store::CoordinatorLeadershipStatus::Active => {
                "active".to_string()
            }
            flowable_engine::persistence::runtime_store::CoordinatorLeadershipStatus::Expired => {
                "expired".to_string()
            }
        },
    }
}

fn timer_node_response(
    node: flowable_engine::persistence::runtime_store::TimerNodeStatus,
) -> TimerNodeResponse {
    TimerNodeResponse {
        node_id: node.node_id,
        worker_type: node.worker_type,
        last_heartbeat: node.last_heartbeat,
        status: match node.status {
            flowable_engine::persistence::runtime_store::NodeStatus::Active => "active".to_string(),
            flowable_engine::persistence::runtime_store::NodeStatus::Expired => {
                "expired".to_string()
            }
        },
    }
}

fn timer_metrics_response(
    metrics: &flowable_engine::engine::timer_worker::TimerCoordinationMetrics,
) -> TimerMetricsResponse {
    TimerMetricsResponse {
        acquire_attempts: metrics.acquire_attempts.load(Ordering::Relaxed),
        acquire_conflicts: metrics.acquire_conflicts.load(Ordering::Relaxed),
        jobs_acquired: metrics.jobs_acquired.load(Ordering::Relaxed),
        last_acquire_batch_size: metrics.last_acquire_batch_size.load(Ordering::Relaxed),
        renew_successes: metrics.renew_successes.load(Ordering::Relaxed),
        renew_misses: metrics.renew_misses.load(Ordering::Relaxed),
        expired_lease_recoveries: metrics.expired_lease_recoveries.load(Ordering::Relaxed),
        execute_count_runtime_job: metrics.execute_count_runtime_job.load(Ordering::Relaxed),
        execute_count_process_start: metrics.execute_count_process_start.load(Ordering::Relaxed),
        execute_count_event_subprocess: metrics
            .execute_count_event_subprocess
            .load(Ordering::Relaxed),
        execute_failures: metrics.execute_failures.load(Ordering::Relaxed),
    }
}

fn directory_provider_name(value: DirectoryProviderKind) -> String {
    match value {
        DirectoryProviderKind::Internal => "internal".to_string(),
        DirectoryProviderKind::LdapMirror => "ldap-mirror".to_string(),
        DirectoryProviderKind::LdapLive => "ldap-live".to_string(),
    }
}

fn operations_exposure_name(value: OperationsExposureKind) -> String {
    match value {
        OperationsExposureKind::MetricsOnly => "metrics-only".to_string(),
        OperationsExposureKind::JmxBridge => "jmx-bridge".to_string(),
        OperationsExposureKind::JmxNativeCompatible => "jmx-native-compatible".to_string(),
    }
}

fn object_family_breadth_name(value: OperationsObjectFamilyBreadth) -> String {
    match value {
        OperationsObjectFamilyBreadth::MetricsSurfacesOnly => "metrics-surfaces-only".to_string(),
        OperationsObjectFamilyBreadth::LedgersOnly => "ledgers-only".to_string(),
        OperationsObjectFamilyBreadth::CoreRuntimeAndPlatformLedgers => {
            "core-runtime-and-platform-ledgers".to_string()
        }
    }
}

fn topology_profile_name(value: CertifiedTopologyProfile) -> String {
    match value {
        CertifiedTopologyProfile::RepositoryDefined => "repository-defined".to_string(),
        CertifiedTopologyProfile::ReverseProxyTerminated => "reverse-proxy-terminated".to_string(),
        CertifiedTopologyProfile::CdiSidecar => "cdi-sidecar".to_string(),
        CertifiedTopologyProfile::OsgiOperationsNode => "osgi-operations-node".to_string(),
        CertifiedTopologyProfile::DeclaredExternal => "declared-external".to_string(),
    }
}

fn runtime_embedding_mode_name(value: RuntimeEmbeddingMode) -> String {
    match value {
        RuntimeEmbeddingMode::Standalone => "standalone".to_string(),
        RuntimeEmbeddingMode::Embedded => "embedded".to_string(),
    }
}

fn runtime_embedding_profile_name(value: RuntimeEmbeddingProfile) -> String {
    match value {
        RuntimeEmbeddingProfile::StandaloneService => "standalone-service".to_string(),
        RuntimeEmbeddingProfile::CdiCompatible => "cdi-compatible".to_string(),
        RuntimeEmbeddingProfile::OsgiManaged => "osgi-managed".to_string(),
    }
}

fn enterprise_adapter_family_name(value: EnterpriseAdapterFamily) -> String {
    match value {
        EnterpriseAdapterFamily::Camel => "camel".to_string(),
        EnterpriseAdapterFamily::Cxf => "cxf".to_string(),
        EnterpriseAdapterFamily::Cdi => "cdi".to_string(),
        EnterpriseAdapterFamily::Osgi => "osgi".to_string(),
    }
}

fn enterprise_support_kind_name(value: EnterpriseSupportKind) -> String {
    match value {
        EnterpriseSupportKind::CompatibilityLayer => "compatibility-layer".to_string(),
        EnterpriseSupportKind::ReplacementArchitecture => "replacement-architecture".to_string(),
    }
}
