use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, Uri},
    routing::{get, post},
};
use chrono::{SecondsFormat, TimeZone, Utc};
use flowable_engine::engine::external_worker_service::{
    ExternalWorkerBpmnErrorRequest as EngineExternalWorkerBpmnErrorRequest,
    ExternalWorkerCmmnTerminateRequest as EngineExternalWorkerCmmnTerminateRequest,
    ExternalWorkerFailureRequest as EngineExternalWorkerFailureRequest,
    ExternalWorkerFetchAndLockRequest as EngineExternalWorkerFetchAndLockRequest,
    ExternalWorkerJob as EngineExternalWorkerJob, ExternalWorkerJobKind,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::{RuntimeStore, RuntimeTimerJobState};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::sync::Arc;

const FETCH_AND_LOCK_PATH: &str = "/external-worker/jobs/fetch-and-lock";
const JOBS_PATH: &str = "/external-worker/jobs";
const JOB_PATH: &str = "/external-worker/jobs/:id";
const COMPLETE_PATH: &str = "/external-worker/jobs/:id/complete";
const FAILURE_PATH: &str = "/external-worker/jobs/:id/failure";
const BPMN_ERROR_PATH: &str = "/external-worker/jobs/:id/bpmnError";
const CMMN_TERMINATE_PATH: &str = "/external-worker/jobs/:id/cmmnTerminate";
const UNLOCK_PATH: &str = "/external-worker/jobs/:id/unlock";
const BULK_UNLOCK_PATH: &str = "/external-worker/jobs/bulk-unlock";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{FETCH_AND_LOCK_PATH}"),
            post(fetch_and_lock),
        )
        .route(&format!("{prefix}{JOBS_PATH}"), get(list))
        .route(&format!("{prefix}{JOB_PATH}"), get(get_job))
        .route(&format!("{prefix}{COMPLETE_PATH}"), post(complete))
        .route(&format!("{prefix}{FAILURE_PATH}"), post(failure))
        .route(&format!("{prefix}{BPMN_ERROR_PATH}"), post(bpmn_error))
        .route(
            &format!("{prefix}{CMMN_TERMINATE_PATH}"),
            post(cmmn_terminate),
        )
        .route(&format!("{prefix}{UNLOCK_PATH}"), post(unlock))
        .route(&format!("{prefix}{BULK_UNLOCK_PATH}"), post(bulk_unacquire))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchAndLockRequest {
    #[serde(rename = "workerId")]
    worker_id: String,
    topic: Option<String>,
    #[serde(rename = "scopeType")]
    scope_type: Option<String>,
    #[serde(rename = "numberOfRetries")]
    number_of_retries: Option<i32>,
    #[serde(rename = "numberOfTasks")]
    number_of_tasks: Option<usize>,
    #[serde(rename = "maxJobs")]
    max_jobs: Option<usize>,
    #[serde(rename = "lockDuration")]
    lock_duration: Option<String>,
    #[serde(rename = "lockDurationMs")]
    lock_duration_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerActionRequest {
    #[serde(rename = "workerId")]
    worker_id: String,
    /// Java `BaseExternalWorkCompletionRequest#variables` — process variable
    /// writeback on complete (`ExternalWorkerJobCompleteCmd.java:75-81`).
    #[serde(default)]
    variables: Option<Vec<CompleteVariable>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteVariable {
    name: String,
    value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BpmnErrorRequest {
    #[serde(rename = "workerId")]
    worker_id: String,
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
    variables: Option<Vec<BpmnErrorVariable>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BpmnErrorVariable {
    name: String,
    value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CmmnTerminateRequest {
    #[serde(rename = "workerId")]
    worker_id: String,
    terminate: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BulkUnacquireRequest {
    #[serde(rename = "workerId")]
    worker_id: String,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailureRequest {
    #[serde(rename = "workerId")]
    worker_id: String,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
    #[serde(rename = "errorDetails")]
    error_details: Option<String>,
    retries: Option<i32>,
    #[serde(rename = "retryTimeout")]
    retry_timeout: Option<String>,
    #[serde(rename = "retryDurationMs")]
    retry_duration_ms: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ExternalWorkerJobListQuery {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    locked: bool,
    unlocked: bool,
}

impl ExternalWorkerJobListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }

    fn validate(&self) -> Result<(), ApiError> {
        if self.locked && self.unlocked {
            return Err(ApiError::bad_request(
                "locked and unlocked cannot both be true",
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ExternalWorkerJobKindResponse {
    RuntimeTimer,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalWorkerJobResponse {
    id: String,
    job_kind: ExternalWorkerJobKindResponse,
    process_instance_id: String,
    process_definition_id: Option<String>,
    execution_id: String,
    element_id: String,
    is_boundary: bool,
    lock_owner: Option<String>,
    due_date: Option<String>,
    lock_expiration_time: Option<String>,
    retries: i32,
    exception_message: Option<String>,
    error_details: Option<String>,
    /// Topic from jobHandlerConfiguration (create-time).
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<String>,
    /// Java `AcquiredExternalWorkerJobResponse#variables` (in-parameter projection).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    variables: Vec<AcquiredVariableResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AcquiredVariableResponse {
    name: String,
    value: serde_json::Value,
}

struct ExternalWorkerJobView {
    id: String,
    job_kind: ExternalWorkerJobKindResponse,
    process_instance_id: String,
    process_definition_id: Option<String>,
    execution_id: String,
    element_id: String,
    is_boundary: bool,
    lock_owner: Option<String>,
    due_date: Option<i64>,
    lock_expiration_time: Option<i64>,
    retries: i32,
    exception_message: Option<String>,
    error_details: Option<String>,
    topic: Option<String>,
    variables: Vec<AcquiredVariableResponse>,
}

impl ExternalWorkerJobView {
    fn from_locked_job(runtime_store: &RuntimeStore, job: EngineExternalWorkerJob) -> Self {
        let variables = job
            .variables
            .into_iter()
            .map(|(name, value)| AcquiredVariableResponse { name, value })
            .collect();
        Self {
            id: job.id,
            job_kind: map_job_kind(job.job_kind),
            process_definition_id: process_definition_id(runtime_store, &job.process_instance_id),
            process_instance_id: job.process_instance_id,
            execution_id: job.execution_id,
            element_id: job.activity_id,
            is_boundary: job.is_boundary,
            lock_owner: non_empty(job.worker_id),
            due_date: job.due_time,
            lock_expiration_time: some_if_positive(job.lock_expiration_time),
            retries: job.retries,
            exception_message: job.error_message,
            error_details: job.error_details,
            topic: job.topic,
            variables,
        }
    }

    fn from_timer_job_state(runtime_store: &RuntimeStore, state: RuntimeTimerJobState) -> Self {
        Self {
            id: state.timer_job_id,
            job_kind: ExternalWorkerJobKindResponse::RuntimeTimer,
            process_definition_id: process_definition_id(runtime_store, &state.process_instance_id),
            process_instance_id: state.process_instance_id,
            execution_id: state.execution_id,
            element_id: state.activity_id,
            is_boundary: state.is_boundary,
            lock_owner: state.lock_owner,
            due_date: state.due_time,
            lock_expiration_time: state.lock_expiration_time,
            retries: state.retries.unwrap_or(1),
            exception_message: state.error_message,
            error_details: state.error_details,
            topic: state.job_handler_configuration,
            variables: Vec::new(),
        }
    }

    fn into_response(self) -> ExternalWorkerJobResponse {
        ExternalWorkerJobResponse {
            id: self.id,
            job_kind: self.job_kind,
            process_instance_id: self.process_instance_id,
            process_definition_id: self.process_definition_id,
            execution_id: self.execution_id,
            element_id: self.element_id,
            is_boundary: self.is_boundary,
            lock_owner: self.lock_owner,
            due_date: self.due_date.and_then(format_timestamp_millis),
            lock_expiration_time: self.lock_expiration_time.and_then(format_timestamp_millis),
            retries: self.retries,
            exception_message: self.exception_message,
            error_details: self.error_details,
            topic: self.topic,
            variables: self.variables,
        }
    }
}

pub(crate) async fn fetch_and_lock(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<Json<Vec<ExternalWorkerJobResponse>>, ApiError> {
    let request: FetchAndLockRequest = parse_json_body(&body)?;
    validate_worker_id(&request.worker_id)?;
    let max_jobs = request.max_jobs.or(request.number_of_tasks).unwrap_or(1);
    let lock_duration_ms = request
        .lock_duration_ms
        .or_else(|| {
            request
                .lock_duration
                .as_deref()
                .and_then(parse_iso_duration_millis)
        })
        .ok_or_else(|| ApiError::bad_request("lockDuration is required"))?;
    let _ = (request.scope_type, request.number_of_retries);

    // Java AcquireExternalWorkerJobRequest.topic is required; when provided we
    // filter. When omitted, keep the unfiltered path for legacy clients/tests.
    let jobs = engine.get_external_worker_service().fetch_and_lock(
        EngineExternalWorkerFetchAndLockRequest {
            worker_id: request.worker_id,
            max_jobs,
            lock_duration_ms,
            topic: request.topic.filter(|t| !t.trim().is_empty()),
        },
    )?;

    let runtime_store = engine.get_runtime_store();
    let response = jobs
        .into_iter()
        .map(|job| ExternalWorkerJobView::from_locked_job(&runtime_store, job).into_response())
        .collect();

    Ok(Json(response))
}

pub(crate) async fn complete(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    body: String,
) -> Result<axum::http::StatusCode, ApiError> {
    let request: WorkerActionRequest = parse_json_body(&body)?;
    validate_worker_id(&request.worker_id)?;

    let variables = request.variables.map(|vars| {
        vars.into_iter()
            .map(|v| (v.name, v.value))
            .collect::<std::collections::HashMap<_, _>>()
    });

    engine
        .get_external_worker_service()
        .complete_with_variables(&id, &request.worker_id, variables)?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(crate) async fn failure(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    body: String,
) -> Result<axum::http::StatusCode, ApiError> {
    let request: FailureRequest = parse_json_body(&body)?;
    validate_worker_id(&request.worker_id)?;
    let retries = request.retries.unwrap_or(0);
    let retry_duration_ms = request
        .retry_duration_ms
        .or_else(|| {
            request
                .retry_timeout
                .as_deref()
                .and_then(parse_iso_duration_millis)
        })
        .unwrap_or(0);

    engine.get_external_worker_service().handle_failure(
        &id,
        EngineExternalWorkerFailureRequest {
            worker_id: request.worker_id,
            error_message: request.error_message,
            error_details: request.error_details,
            retries,
            retry_duration_ms,
        },
    )?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(crate) async fn unlock(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    body: String,
) -> Result<axum::http::StatusCode, ApiError> {
    let request: WorkerActionRequest = parse_json_body(&body)?;
    validate_worker_id(&request.worker_id)?;

    engine
        .get_external_worker_service()
        .unlock(&id, &request.worker_id)?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(crate) async fn bpmn_error(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: BpmnErrorRequest = parse_json_body(&body)?;
    validate_worker_id(&request.worker_id)?;
    let error_code = request
        .error_code
        .ok_or_else(|| ApiError::bad_request("errorCode is required"))?;

    let BpmnErrorRequest {
        worker_id,
        error_message,
        variables,
        ..
    } = request;

    let variables = variables.map(|vars| {
        vars.into_iter()
            .map(|v| (v.name, v.value))
            .collect::<std::collections::HashMap<_, _>>()
    });

    engine
        .get_external_worker_service()
        .complete_with_bpmn_error(
            &id,
            EngineExternalWorkerBpmnErrorRequest {
                worker_id,
                error_code,
                error_message,
                variables,
            },
        )?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn cmmn_terminate(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: CmmnTerminateRequest = parse_json_body(&body)?;
    validate_worker_id(&request.worker_id)?;

    engine.get_external_worker_service().cmmn_terminate(
        &id,
        EngineExternalWorkerCmmnTerminateRequest {
            worker_id: request.worker_id,
            terminate: request.terminate.unwrap_or(true),
        },
    )?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn bulk_unacquire(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: BulkUnacquireRequest = parse_json_body(&body)?;
    validate_worker_id(&request.worker_id)?;
    let _ = request.tenant_id;

    let runtime_store = engine.get_runtime_store();
    let now = runtime_store.time_source().now().timestamp_millis();
    let mut session = runtime_store.create_session().unwrap();
    let job_ids = runtime_store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|state| state.lock_owner.as_deref() == Some(request.worker_id.as_str()))
        .filter(|state| state.lock_expiration_time.unwrap_or_default() > now)
        .map(|state| state.timer_job_id)
        .collect::<Vec<_>>();

    for job_id in job_ids {
        engine
            .get_external_worker_service()
            .unlock(&job_id, &request.worker_id)?;
    }

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ExternalWorkerJobResponse>>, ApiError> {
    let query: ExternalWorkerJobListQuery = parse_query(&uri)?;
    query.validate()?;

    let runtime_store = engine.get_runtime_store();
    let now = runtime_store.time_source().now().timestamp_millis();
    // Family isolation is owned by the engine service (externalWorker + timer +
    // parent active). locked/unlocked filters apply only on that result set.
    let mut jobs: Vec<_> = engine
        .get_external_worker_service()
        .list_active_timer_jobs()
        .into_iter()
        .filter(|state| filter_job(state, &query, now))
        .map(|state| {
            ExternalWorkerJobView::from_timer_job_state(&runtime_store, state).into_response()
        })
        .collect();

    jobs.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Json(query.paging().paginate(jobs)))
}

pub(crate) async fn get_job(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
) -> Result<Json<ExternalWorkerJobResponse>, ApiError> {
    let runtime_store = engine.get_runtime_store();
    engine
        .get_external_worker_service()
        .find_active_timer_job(&id)
        .map(|state| {
            Json(ExternalWorkerJobView::from_timer_job_state(&runtime_store, state).into_response())
        })
        .ok_or_else(|| ApiError::NotFound(format!("External worker job '{}' was not found", id)))
}

fn parse_json_body<T>(body: &str) -> Result<T, ApiError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(body).map_err(|err| ApiError::bad_request(err.to_string()))
}

fn validate_worker_id(worker_id: &str) -> Result<(), ApiError> {
    if worker_id.trim().is_empty() {
        return Err(ApiError::bad_request("workerId is required"));
    }

    Ok(())
}

fn parse_iso_duration_millis(value: &str) -> Option<i64> {
    let rest = value.strip_prefix("PT")?;
    if rest.is_empty() {
        return None;
    }

    let mut number = String::new();
    let mut total_millis: i64 = 0;
    for ch in rest.chars() {
        if ch.is_ascii_digit() {
            number.push(ch);
            continue;
        }

        let amount: i64 = number.parse().ok()?;
        number.clear();
        let multiplier = match ch {
            'H' => 60 * 60 * 1_000,
            'M' => 60 * 1_000,
            'S' => 1_000,
            _ => return None,
        };
        total_millis = total_millis.checked_add(amount.checked_mul(multiplier)?)?;
    }

    if number.is_empty() && total_millis > 0 {
        Some(total_millis)
    } else {
        None
    }
}

fn process_definition_id(
    runtime_store: &RuntimeStore,
    process_instance_id: &str,
) -> Option<String> {
    let mut session = runtime_store.create_session().unwrap();
    runtime_store
        .find_process_instance(process_instance_id, &mut session)
        .map(|instance| instance.process_definition_id)
}

fn map_job_kind(job_kind: ExternalWorkerJobKind) -> ExternalWorkerJobKindResponse {
    match job_kind {
        ExternalWorkerJobKind::RuntimeTimer => ExternalWorkerJobKindResponse::RuntimeTimer,
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn some_if_positive(value: i64) -> Option<i64> {
    if value > 0 { Some(value) } else { None }
}

fn format_timestamp_millis(timestamp_millis: i64) -> Option<String> {
    Utc.timestamp_millis_opt(timestamp_millis)
        .single()
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn filter_job(state: &RuntimeTimerJobState, query: &ExternalWorkerJobListQuery, now: i64) -> bool {
    if let Some(id) = query.id.as_deref()
        && state.timer_job_id != id
    {
        return false;
    }

    if let Some(process_instance_id) = query.process_instance_id.as_deref()
        && state.process_instance_id != process_instance_id
    {
        return false;
    }

    if let Some(execution_id) = query.execution_id.as_deref()
        && state.execution_id != execution_id
    {
        return false;
    }

    let locked = is_locked(state, now);
    if query.locked && !locked {
        return false;
    }
    if query.unlocked && locked {
        return false;
    }

    true
}

fn is_locked(state: &RuntimeTimerJobState, now: i64) -> bool {
    matches!(
        (state.lock_owner.as_deref(), state.lock_expiration_time),
        (Some(_), Some(lock_expiration_time)) if lock_expiration_time > now
    )
}
