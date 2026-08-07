use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use crate::query_variable::{
    QueryVariableOperation, validate_name_less_equals, validate_operation_value, value_matches,
};
use axum::{
    Extension, Json, Router,
    http::{HeaderMap, StatusCode, Uri, header},
    routing::get,
};
use axum::{extract::Path, routing::post};
use base64::Engine as _;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::history::historic_entities::{
    HistoricActivityInstance, HistoricDetail, HistoricIdentityLink, HistoricProcessInstance,
    HistoricTaskInstance, HistoricTaskLogEntry, HistoricVariableInstance,
};
use flowable_engine::identity::entities::IdentityLink;
use flowable_form_service::{FlowableFormService, FormInstance};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::variable_types::rest_variable_type;

const HISTORIC_PROCESS_INSTANCES_PATH: &str = "/history/historic-process-instances";
const HISTORIC_PROCESS_INSTANCES_DELETE_PATH: &str = "/history/historic-process-instances/delete";
const HISTORIC_PROCESS_INSTANCE_PATH: &str =
    "/history/historic-process-instances/:process_instance_id";
const HISTORIC_PROCESS_INSTANCE_IDENTITY_LINKS_PATH: &str =
    "/history/historic-process-instances/:process_instance_id/identitylinks";
const HISTORIC_PROCESS_INSTANCE_COMMENTS_PATH: &str =
    "/history/historic-process-instances/:process_instance_id/comments";
const HISTORIC_PROCESS_INSTANCE_COMMENT_PATH: &str =
    "/history/historic-process-instances/:process_instance_id/comments/:comment_id";
const HISTORIC_PROCESS_INSTANCE_VARIABLE_DATA_PATH: &str =
    "/history/historic-process-instances/:process_instance_id/variables/:variable_name/data";
const HISTORIC_PROCESS_INSTANCES_QUERY_PATH: &str = "/query/historic-process-instances";
const HISTORIC_DETAILS_PATH: &str = "/history/historic-detail";
const HISTORIC_DETAIL_DATA_PATH: &str = "/history/historic-detail/:detail_id/data";
const HISTORIC_DETAILS_QUERY_PATH: &str = "/query/historic-detail";
const HISTORIC_TASK_INSTANCES_PATH: &str = "/history/historic-task-instances";
const HISTORIC_TASK_INSTANCES_DELETE_PATH: &str = "/history/historic-task-instances/delete";
const HISTORIC_TASK_INSTANCE_PATH: &str = "/history/historic-task-instances/:task_id";
const HISTORIC_TASK_INSTANCE_IDENTITY_LINKS_PATH: &str =
    "/history/historic-task-instances/:task_id/identitylinks";
const HISTORIC_TASK_INSTANCE_FORM_PATH: &str = "/history/historic-task-instances/:task_id/form";
const HISTORIC_TASK_INSTANCE_VARIABLE_DATA_PATH: &str =
    "/history/historic-task-instances/:task_id/variables/:variable_name/data";
const HISTORIC_TASK_LOG_ENTRIES_PATH: &str = "/history/historic-task-log-entries";
const HISTORIC_ACTIVITY_INSTANCES_PATH: &str = "/history/historic-activity-instances";
const HISTORIC_TASK_INSTANCES_QUERY_PATH: &str = "/query/historic-task-instances";
const HISTORIC_ACTIVITY_INSTANCES_QUERY_PATH: &str = "/query/historic-activity-instances";
const HISTORIC_VARIABLE_INSTANCES_QUERY_PATH: &str = "/query/historic-variable-instances";
const HISTORIC_VARIABLE_INSTANCES_PATH: &str = "/history/historic-variable-instances";
const HISTORIC_VARIABLE_INSTANCE_DATA_PATH: &str =
    "/history/historic-variable-instances/:variable_instance_id/data";
const HISTORY_CLEANUP_PATH: &str = "/history/history-cleanup";
const HISTORY_CLEANUP_STRATEGY_PATH: &str = "/history/history-cleanup/strategy";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{HISTORIC_PROCESS_INSTANCES_PATH}"),
            get(historic_process_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_PROCESS_INSTANCES_DELETE_PATH}"),
            post(bulk_delete_historic_process_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_PROCESS_INSTANCE_PATH}"),
            get(get_historic_process_instance).delete(delete_historic_process_instance),
        )
        .route(
            &format!("{prefix}{HISTORIC_PROCESS_INSTANCE_IDENTITY_LINKS_PATH}"),
            get(get_historic_process_instance_identity_links),
        )
        .route(
            &format!("{prefix}{HISTORIC_PROCESS_INSTANCE_COMMENTS_PATH}"),
            get(get_historic_process_instance_comments)
                .post(create_historic_process_instance_comment),
        )
        .route(
            &format!("{prefix}{HISTORIC_PROCESS_INSTANCE_COMMENT_PATH}"),
            get(get_historic_process_instance_comment)
                .delete(delete_historic_process_instance_comment),
        )
        .route(
            &format!("{prefix}{HISTORIC_PROCESS_INSTANCE_VARIABLE_DATA_PATH}"),
            get(get_historic_process_instance_variable_data),
        )
        .route(
            &format!("{prefix}{HISTORIC_PROCESS_INSTANCES_QUERY_PATH}"),
            post(query_historic_process_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_DETAILS_PATH}"),
            get(historic_details),
        )
        .route(
            &format!("{prefix}{HISTORIC_DETAIL_DATA_PATH}"),
            get(get_historic_detail_data),
        )
        .route(
            &format!("{prefix}{HISTORIC_DETAILS_QUERY_PATH}"),
            post(query_historic_details),
        )
        .route(
            &format!("{prefix}{HISTORIC_TASK_INSTANCES_PATH}"),
            get(historic_task_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_TASK_INSTANCES_DELETE_PATH}"),
            post(bulk_delete_historic_task_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_TASK_INSTANCE_PATH}"),
            get(get_historic_task_instance).delete(delete_historic_task_instance),
        )
        .route(
            &format!("{prefix}{HISTORIC_TASK_INSTANCE_IDENTITY_LINKS_PATH}"),
            get(get_historic_task_instance_identity_links),
        )
        .route(
            &format!("{prefix}{HISTORIC_TASK_INSTANCE_FORM_PATH}"),
            get(get_historic_task_instance_form),
        )
        .route(
            &format!("{prefix}{HISTORIC_TASK_INSTANCE_VARIABLE_DATA_PATH}"),
            get(get_historic_task_instance_variable_data),
        )
        .route(
            &format!("{prefix}{HISTORIC_TASK_LOG_ENTRIES_PATH}"),
            get(historic_task_log_entries),
        )
        .route(
            &format!("{prefix}{HISTORIC_ACTIVITY_INSTANCES_PATH}"),
            get(historic_activity_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_TASK_INSTANCES_QUERY_PATH}"),
            post(query_historic_task_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_ACTIVITY_INSTANCES_QUERY_PATH}"),
            post(query_historic_activity_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_VARIABLE_INSTANCES_QUERY_PATH}"),
            post(query_historic_variable_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_VARIABLE_INSTANCES_PATH}"),
            get(historic_variable_instances),
        )
        .route(
            &format!("{prefix}{HISTORIC_VARIABLE_INSTANCE_DATA_PATH}"),
            get(get_historic_variable_instance_data),
        )
        .route(
            &format!("{prefix}{HISTORY_CLEANUP_PATH}"),
            post(cleanup_history),
        )
        .route(
            &format!("{prefix}{HISTORY_CLEANUP_STRATEGY_PATH}"),
            post(configure_cleanup_strategy),
        )
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HistoricProcessInstanceListQuery {
    start: usize,
    size: Option<usize>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "processInstanceIds")]
    process_instance_ids: Option<Vec<String>>,
    #[serde(rename = "processInstanceName")]
    process_instance_name: Option<String>,
    #[serde(rename = "processInstanceNameLike")]
    process_instance_name_like: Option<String>,
    #[serde(rename = "processInstanceNameLikeIgnoreCase")]
    process_instance_name_like_ignore_case: Option<String>,
    #[serde(rename = "processDefinitionId")]
    process_definition_id: Option<String>,
    #[serde(rename = "processDefinitionKey")]
    process_definition_key: Option<String>,
    #[serde(rename = "processDefinitionKeyLike")]
    process_definition_key_like: Option<String>,
    #[serde(rename = "processDefinitionKeyLikeIgnoreCase")]
    process_definition_key_like_ignore_case: Option<String>,
    #[serde(rename = "processDefinitionKeys", alias = "processDefinitionKeyIn")]
    process_definition_keys: Option<Vec<String>>,
    #[serde(
        rename = "excludeProcessDefinitionKeys",
        alias = "processDefinitionKeyNotIn"
    )]
    exclude_process_definition_keys: Option<Vec<String>>,
    #[serde(rename = "processDefinitionName")]
    process_definition_name: Option<String>,
    #[serde(rename = "processDefinitionNameLike")]
    process_definition_name_like: Option<String>,
    #[serde(rename = "processDefinitionNameLikeIgnoreCase")]
    process_definition_name_like_ignore_case: Option<String>,
    #[serde(rename = "processDefinitionVersion")]
    process_definition_version: Option<i32>,
    #[serde(rename = "processDefinitionCategory")]
    process_definition_category: Option<String>,
    #[serde(rename = "processDefinitionCategoryLike")]
    process_definition_category_like: Option<String>,
    #[serde(rename = "processDefinitionCategoryLikeIgnoreCase")]
    process_definition_category_like_ignore_case: Option<String>,
    #[serde(rename = "deploymentId")]
    deployment_id: Option<String>,
    #[serde(rename = "deploymentIdIn")]
    deployment_id_in: Option<Vec<String>>,
    #[serde(
        rename = "businessKey",
        alias = "processInstanceBusinessKey",
        alias = "processBusinessKey"
    )]
    business_key: Option<String>,
    #[serde(
        rename = "businessKeyLike",
        alias = "processInstanceBusinessKeyLike",
        alias = "processBusinessKeyLike"
    )]
    business_key_like: Option<String>,
    #[serde(
        rename = "businessKeyLikeIgnoreCase",
        alias = "processInstanceBusinessKeyLikeIgnoreCase",
        alias = "processBusinessKeyLikeIgnoreCase"
    )]
    business_key_like_ignore_case: Option<String>,
    #[serde(rename = "businessStatus", alias = "processBusinessStatus")]
    business_status: Option<String>,
    #[serde(rename = "businessStatusLike", alias = "processBusinessStatusLike")]
    business_status_like: Option<String>,
    #[serde(
        rename = "businessStatusLikeIgnoreCase",
        alias = "processBusinessStatusLikeIgnoreCase"
    )]
    business_status_like_ignore_case: Option<String>,
    finished: Option<bool>,
    unfinished: Option<bool>,
    #[serde(rename = "startedBy")]
    started_by: Option<String>,
    #[serde(rename = "finishedBy")]
    finished_by: Option<String>,
    state: Option<String>,
    #[serde(rename = "superProcessInstanceId")]
    super_process_instance_id: Option<String>,
    #[serde(rename = "excludeSubprocesses")]
    exclude_subprocesses: Option<bool>,
    #[serde(rename = "activeActivityId")]
    active_activity_id: Option<String>,
    #[serde(rename = "activeActivityIds")]
    active_activity_ids: Option<Vec<String>>,
    #[serde(rename = "involvedUser")]
    involved_user: Option<String>,
    #[serde(rename = "callbackId")]
    callback_id: Option<String>,
    #[serde(rename = "callbackIds")]
    callback_ids: Option<Vec<String>>,
    #[serde(rename = "callbackType")]
    callback_type: Option<String>,
    #[serde(rename = "withoutCallbackId")]
    without_callback_id: Option<bool>,
    #[serde(rename = "parentCaseInstanceId")]
    parent_case_instance_id: Option<String>,
    #[serde(rename = "rootScopeId")]
    root_scope_id: Option<String>,
    #[serde(rename = "parentScopeId")]
    parent_scope_id: Option<String>,
    #[serde(rename = "startedAfter")]
    started_after: Option<String>,
    #[serde(rename = "startedBefore")]
    started_before: Option<String>,
    #[serde(rename = "finishedAfter")]
    finished_after: Option<String>,
    #[serde(rename = "finishedBefore")]
    finished_before: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    #[serde(rename = "tenantIdLikeIgnoreCase")]
    tenant_id_like_ignore_case: Option<String>,
    #[serde(rename = "withoutTenantId")]
    without_tenant_id: Option<bool>,
    #[serde(rename = "includeProcessVariables")]
    include_process_variables: Option<bool>,
    #[serde(rename = "includeProcessVariablesNames")]
    include_process_variables_names: Option<Vec<String>>,
    variables: Option<Vec<HistoricQueryVariable>>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkDeleteHistoricProcessInstancesRequest {
    action: Option<String>,
    #[serde(default)]
    instance_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkDeleteHistoricTaskInstancesRequest {
    action: Option<String>,
    #[serde(default, alias = "historicTaskInstanceIds")]
    task_instance_ids: Vec<String>,
}

impl HistoricProcessInstanceListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct HistoricProcessInstanceResponse {
    pub id: String,
    #[serde(rename = "processDefinitionId")]
    pub process_definition_id: String,
    #[serde(rename = "processDefinitionKey")]
    pub process_definition_key: Option<String>,
    #[serde(rename = "businessKey")]
    pub business_key: Option<String>,
    #[serde(rename = "startTime")]
    pub start_time: String,
    #[serde(rename = "endTime")]
    pub end_time: Option<String>,
    #[serde(rename = "durationInMillis")]
    pub duration_in_millis: Option<i64>,
    #[serde(rename = "tenantId")]
    pub tenant_id: Option<String>,
    #[serde(rename = "deleteReason")]
    pub delete_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<Vec<super::process_instances::RestVariableResponse>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricTaskFormResponse {
    pub id: String,
    pub form_definition_id: String,
    pub form_definition_key: String,
    pub form_definition_name: String,
    pub deployment_id: String,
    pub process_definition_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub task_id: Option<String>,
    pub submitted_at: i64,
    pub values: BTreeMap<String, Value>,
}

fn to_historic_process_instance_response(
    instance: HistoricProcessInstance,
    definitions: &HashMap<String, HistoricProcessDefinitionMeta>,
) -> HistoricProcessInstanceResponse {
    let definition = definitions.get(&instance.process_definition_id);
    HistoricProcessInstanceResponse {
        id: instance.id.clone(),
        process_definition_id: instance.process_definition_id.clone(),
        process_definition_key: historic_process_definition_key(&instance, definitions),
        business_key: instance.business_key.clone(),
        start_time: instance.start_time.to_rfc3339(),
        end_time: instance.end_time.map(|time| time.to_rfc3339()),
        duration_in_millis: instance.duration_ms,
        tenant_id: definition.and_then(|definition| definition.tenant_id.clone()),
        delete_reason: instance.delete_reason,
        variables: None,
    }
}

pub(crate) async fn historic_process_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricProcessInstanceResponse>>, ApiError> {
    let query: HistoricProcessInstanceListQuery = parse_query(&uri)?;
    let instances = historic_process_instances_for_query(Arc::clone(&engine), &query)?;

    Ok(Json(paginate_historic_process_instances(
        &engine, query, instances,
    )?))
}

pub(crate) async fn get_historic_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<Json<HistoricProcessInstanceResponse>, ApiError> {
    let instance = engine
        .get_history_service()
        .create_historic_process_instance_query()
        .process_instance_id(process_instance_id.clone())
        .single_result()?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic process instance '{}' was not found",
                process_instance_id
            ))
        })?;

    let definitions = historic_process_definition_meta(&engine)?;
    Ok(Json(to_historic_process_instance_response(
        instance,
        &definitions,
    )))
}

pub(crate) async fn delete_historic_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_history_service()
        .delete_historic_process_instance(process_instance_id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn bulk_delete_historic_process_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let payload: BulkDeleteHistoricProcessInstancesRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    match payload.action.as_deref() {
        Some("delete") => {}
        Some(action) => {
            return Err(ApiError::BadRequest(format!("Illegal action: '{action}'.")));
        }
        None => {
            return Err(ApiError::BadRequest(
                "Illegal action: action is required.".to_string(),
            ));
        }
    }
    if payload.instance_ids.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one historic process instance id is required.".to_string(),
        ));
    }

    engine
        .get_history_service()
        .bulk_delete_historic_process_instances(payload.instance_ids)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn bulk_delete_historic_task_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let payload: BulkDeleteHistoricTaskInstancesRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    match payload.action.as_deref() {
        Some("delete") => {}
        Some(action) => {
            return Err(ApiError::BadRequest(format!("Illegal action: '{action}'.")));
        }
        None => {
            return Err(ApiError::BadRequest(
                "Illegal action: action is required.".to_string(),
            ));
        }
    }
    if payload.task_instance_ids.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one historic task instance id is required.".to_string(),
        ));
    }

    for task_id in payload.task_instance_ids {
        engine
            .get_history_service()
            .delete_historic_task_instance(task_id)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn ensure_historic_process_instance_exists(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Result<(), ApiError> {
    engine
        .get_history_service()
        .create_historic_process_instance_query()
        .process_instance_id(process_instance_id.to_string())
        .single_result()?
        .map(|_| ())
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic process instance '{}' was not found",
                process_instance_id
            ))
        })
}

pub(crate) async fn get_historic_process_instance_identity_links(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<Json<Vec<HistoricIdentityLinkResponse>>, ApiError> {
    ensure_historic_process_instance_exists(&engine, &process_instance_id)?;
    // P77: Java HistoryService.getHistoricIdentityLinksForProcessInstance
    // reads ACT_HI_IDENTITYLINK, not runtime identity links.
    let links = engine
        .get_history_service()
        .get_historic_identity_links_for_process_instance(&process_instance_id)
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?
        .into_iter()
        .map(HistoricIdentityLinkResponse::from)
        .collect();

    Ok(Json(links))
}

pub(crate) async fn get_historic_process_instance_comments(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<Json<Vec<super::tasks::TaskCommentResponse>>, ApiError> {
    ensure_historic_process_instance_exists(&engine, &process_instance_id)?;
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let comments = engine
        .get_history_service()
        .get_process_instance_comments(&process_instance_id, &mut session)
        .into_iter()
        .map(super::tasks::task_comment_response)
        .collect();

    Ok(Json(comments))
}

pub(crate) async fn get_historic_process_instance_comment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, comment_id)): Path<(String, String)>,
) -> Result<Json<super::tasks::TaskCommentResponse>, ApiError> {
    ensure_historic_process_instance_exists(&engine, &process_instance_id)?;
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let comment = engine
        .get_history_service()
        .get_comment(&comment_id, &mut session)
        .filter(|comment| {
            comment.process_instance_id.as_deref() == Some(process_instance_id.as_str())
        })
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic process instance '{}' comment '{}' was not found",
                process_instance_id, comment_id
            ))
        })?;

    Ok(Json(super::tasks::task_comment_response(comment)))
}

/// Java `CommentRequest` as consumed by
/// `HistoricProcessInstanceCommentCollectionResource` (unknown body fields are
/// ignored, matching the Jackson binding used by the Java resource).
#[derive(Debug, Deserialize)]
struct CreateHistoricProcessInstanceCommentRequest {
    message: Option<String>,
}

pub(crate) async fn create_historic_process_instance_comment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<(StatusCode, Json<super::tasks::TaskCommentResponse>), ApiError> {
    ensure_historic_process_instance_exists(&engine, &process_instance_id)?;
    let request: CreateHistoricProcessInstanceCommentRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let message = request
        .message
        .ok_or_else(|| ApiError::BadRequest("Comment text is required.".to_string()))?;
    let author = user_id_from_basic_auth(&headers);
    let comment = engine
        .get_history_service()
        .create_process_instance_comment(&process_instance_id, &message, author.as_deref())?;

    Ok((
        StatusCode::CREATED,
        Json(super::tasks::task_comment_response(comment)),
    ))
}

pub(crate) async fn delete_historic_process_instance_comment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, comment_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_historic_process_instance_exists(&engine, &process_instance_id)?;
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let owned = engine
        .get_history_service()
        .get_comment(&comment_id, &mut session)
        .is_some_and(|comment| {
            comment.process_instance_id.as_deref() == Some(process_instance_id.as_str())
        });
    if !owned {
        return Err(ApiError::NotFound(format!(
            "Historic process instance '{}' comment '{}' was not found",
            process_instance_id, comment_id
        )));
    }
    engine.get_history_service().delete_comment(&comment_id)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Same extraction as the private `tasks::user_id_from_basic_auth`: Java
/// resolves the comment author via `Authentication.getAuthenticatedUserId()`.
fn user_id_from_basic_auth(headers: &HeaderMap) -> Option<String> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())?;
    let encoded = auth_header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let user_id = decoded.split_once(':')?.0.trim();
    if user_id.is_empty() {
        None
    } else {
        Some(user_id.to_string())
    }
}

pub(crate) async fn get_historic_process_instance_variable_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    engine
        .get_history_service()
        .create_historic_process_instance_query()
        .process_instance_id(process_instance_id.clone())
        .single_result()?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic process instance '{}' was not found",
                process_instance_id
            ))
        })?;
    let variable = engine
        .get_history_service()
        .create_historic_variable_instance_query()
        .process_instance_id(process_instance_id.clone())
        .variable_name(variable_name.clone())
        .single_result()?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic variable '{}' was not found for process instance '{}'",
                variable_name, process_instance_id
            ))
        })?;

    Ok(Json(variable.value))
}

pub(crate) async fn get_historic_task_instance_variable_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((task_id, variable_name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let task = load_historic_task_instance(&engine, &task_id)?;
    let variable = engine
        .get_history_service()
        .create_historic_variable_instance_query()
        .process_instance_id(task.process_instance_id.clone())
        .variable_name(variable_name.clone())
        .single_result()?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic variable '{}' was not found for historic task instance '{}'",
                variable_name, task_id
            ))
        })?;

    Ok(Json(variable.value))
}

pub(crate) async fn query_historic_process_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<HistoricProcessInstanceResponse>>, ApiError> {
    let mut query: HistoricProcessInstanceListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: HistoricProcessInstanceListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);
    query.include_process_variables = url_query
        .include_process_variables
        .or(query.include_process_variables);
    let instances = historic_process_instances_for_query(Arc::clone(&engine), &query)?;

    Ok(Json(paginate_historic_process_instances(
        &engine, query, instances,
    )?))
}

fn historic_process_instances_for_query(
    engine: Arc<ProcessEngine>,
    query: &HistoricProcessInstanceListQuery,
) -> Result<Vec<HistoricProcessInstance>, ApiError> {
    let definitions = historic_process_definition_meta(&engine)?;
    let mut instances = engine
        .get_history_service()
        .create_historic_process_instance_query()
        .list()?;

    if let Some(process_instance_id) = query.process_instance_id.as_deref() {
        instances.retain(|instance| instance.id == process_instance_id);
    }
    if let Some(process_definition_id) = query.process_definition_id.as_deref() {
        instances.retain(|instance| instance.process_definition_id == process_definition_id);
    }
    if let Some(process_definition_key) = query.process_definition_key.as_deref() {
        instances.retain(|instance| {
            historic_process_definition_key(instance, &definitions).as_deref()
                == Some(process_definition_key)
        });
    }
    if let Some(process_definition_key_like) = query.process_definition_key_like.as_deref() {
        instances.retain(|instance| {
            historic_process_definition_key(instance, &definitions)
                .as_deref()
                .is_some_and(|key| sql_like_matches(process_definition_key_like, key))
        });
    }
    if let Some(business_key) = query.business_key.as_deref() {
        instances.retain(|instance| instance.business_key.as_deref() == Some(business_key));
    }
    if let Some(business_key_like) = query.business_key_like.as_deref() {
        instances.retain(|instance| {
            instance
                .business_key
                .as_deref()
                .is_some_and(|business_key| sql_like_matches(business_key_like, business_key))
        });
    }
    if let Some(finished) = query.finished {
        instances.retain(|instance| instance.end_time.is_some() == finished);
    }
    if query.unfinished == Some(true) {
        instances.retain(|instance| instance.end_time.is_none());
    }
    if let Some(started_after) = query.started_after.as_deref() {
        let started_after = parse_timestamp_millis(started_after)?;
        instances.retain(|instance| instance.start_time.timestamp_millis() >= started_after);
    }
    if let Some(started_before) = query.started_before.as_deref() {
        let started_before = parse_timestamp_millis(started_before)?;
        instances.retain(|instance| instance.start_time.timestamp_millis() <= started_before);
    }
    if let Some(finished_after) = query.finished_after.as_deref() {
        let finished_after = parse_timestamp_millis(finished_after)?;
        instances.retain(|instance| {
            instance
                .end_time
                .is_some_and(|end_time| end_time.timestamp_millis() >= finished_after)
        });
    }
    if let Some(finished_before) = query.finished_before.as_deref() {
        let finished_before = parse_timestamp_millis(finished_before)?;
        instances.retain(|instance| {
            instance
                .end_time
                .is_some_and(|end_time| end_time.timestamp_millis() <= finished_before)
        });
    }
    if let Some(tenant_id) = query.tenant_id.as_deref() {
        instances.retain(|instance| {
            historic_process_tenant_id(instance, &definitions).as_deref() == Some(tenant_id)
        });
    }
    if let Some(tenant_id_like) = query.tenant_id_like.as_deref() {
        instances.retain(|instance| {
            historic_process_tenant_id(instance, &definitions)
                .as_deref()
                .is_some_and(|tenant_id| sql_like_matches(tenant_id_like, tenant_id))
        });
    }
    if query.without_tenant_id == Some(true) {
        instances.retain(|instance| historic_process_tenant_id(instance, &definitions).is_none());
    }
    if let Some(tenant_id_like_ignore_case) = query.tenant_id_like_ignore_case.as_deref() {
        let pattern = tenant_id_like_ignore_case.to_lowercase();
        instances.retain(|instance| {
            historic_process_tenant_id(instance, &definitions)
                .is_some_and(|tenant_id| sql_like_matches(&pattern, &tenant_id.to_lowercase()))
        });
    }
    if let Some(ids) = query
        .process_instance_ids
        .as_ref()
        .filter(|ids| !ids.is_empty())
    {
        instances.retain(|instance| ids.contains(&instance.id));
    }
    if let Some(keys) = query
        .process_definition_keys
        .as_ref()
        .filter(|keys| !keys.is_empty())
    {
        instances.retain(|instance| {
            historic_process_definition_key(instance, &definitions)
                .is_some_and(|key| keys.contains(&key))
        });
    }
    if let Some(keys) = query
        .exclude_process_definition_keys
        .as_ref()
        .filter(|keys| !keys.is_empty())
    {
        instances.retain(|instance| {
            !historic_process_definition_key(instance, &definitions)
                .is_some_and(|key| keys.contains(&key))
        });
    }
    if let Some(pattern) = query.process_definition_key_like_ignore_case.as_deref() {
        let pattern = pattern.to_lowercase();
        instances.retain(|instance| {
            historic_process_definition_key(instance, &definitions)
                .is_some_and(|key| sql_like_matches(&pattern, &key.to_lowercase()))
        });
    }
    if let Some(name) = query.process_definition_name.as_deref() {
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .is_some_and(|definition| definition.name.as_deref() == Some(name))
        });
    }
    if let Some(pattern) = query.process_definition_name_like.as_deref() {
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .and_then(|definition| definition.name.as_deref())
                .is_some_and(|name| sql_like_matches(pattern, name))
        });
    }
    if let Some(pattern) = query.process_definition_name_like_ignore_case.as_deref() {
        let pattern = pattern.to_lowercase();
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .and_then(|definition| definition.name.as_deref())
                .is_some_and(|name| sql_like_matches(&pattern, &name.to_lowercase()))
        });
    }
    if let Some(version) = query.process_definition_version {
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .is_some_and(|definition| definition.version == version)
        });
    }
    if let Some(category) = query.process_definition_category.as_deref() {
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .is_some_and(|definition| definition.category.as_deref() == Some(category))
        });
    }
    if let Some(pattern) = query.process_definition_category_like.as_deref() {
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .and_then(|definition| definition.category.as_deref())
                .is_some_and(|category| sql_like_matches(pattern, category))
        });
    }
    if let Some(pattern) = query
        .process_definition_category_like_ignore_case
        .as_deref()
    {
        let pattern = pattern.to_lowercase();
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .and_then(|definition| definition.category.as_deref())
                .is_some_and(|category| sql_like_matches(&pattern, &category.to_lowercase()))
        });
    }
    if let Some(deployment_id) = query.deployment_id.as_deref() {
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .is_some_and(|definition| {
                    definition.deployment_id.as_deref() == Some(deployment_id)
                })
        });
    }
    if let Some(deployment_ids) = query
        .deployment_id_in
        .as_ref()
        .filter(|ids| !ids.is_empty())
    {
        instances.retain(|instance| {
            definitions
                .get(&instance.process_definition_id)
                .and_then(|definition| definition.deployment_id.clone())
                .is_some_and(|deployment_id| deployment_ids.contains(&deployment_id))
        });
    }
    if let Some(pattern) = query.business_key_like_ignore_case.as_deref() {
        let pattern = pattern.to_lowercase();
        instances.retain(|instance| {
            instance
                .business_key
                .as_deref()
                .is_some_and(|business_key| {
                    sql_like_matches(&pattern, &business_key.to_lowercase())
                })
        });
    }
    if let Some(started_by) = query.started_by.as_deref() {
        instances.retain(|instance| instance.start_user_id.as_deref() == Some(started_by));
    }
    if query.finished_by.is_some() {
        // Data limitation: the engine records no end-user id on historic
        // process instances (Java END_USER_ID_), so finishedBy never matches.
        instances.clear();
    }
    if query.parent_case_instance_id.is_some() {
        // Data limitation: the BPMN engine holds no CMMN case instance data,
        // so parentCaseInstanceId never matches.
        instances.clear();
    }

    let runtime_needed = query.process_instance_name.is_some()
        || query.process_instance_name_like.is_some()
        || query.process_instance_name_like_ignore_case.is_some()
        || query.business_status.is_some()
        || query.business_status_like.is_some()
        || query.business_status_like_ignore_case.is_some()
        || query.callback_id.is_some()
        || query.callback_ids.is_some()
        || query.callback_type.is_some()
        || query.without_callback_id == Some(true)
        || query.root_scope_id.is_some()
        || query.state.is_some();
    let runtime = if runtime_needed {
        runtime_process_instances_by_id(&engine)
    } else {
        HashMap::new()
    };
    if let Some(name) = query.process_instance_name.as_deref() {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .is_some_and(|pi| pi.name.as_deref() == Some(name))
        });
    }
    if let Some(pattern) = query.process_instance_name_like.as_deref() {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .and_then(|pi| pi.name.as_deref())
                .is_some_and(|name| sql_like_matches(pattern, name))
        });
    }
    if let Some(pattern) = query.process_instance_name_like_ignore_case.as_deref() {
        let pattern = pattern.to_lowercase();
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .and_then(|pi| pi.name.as_deref())
                .is_some_and(|name| sql_like_matches(&pattern, &name.to_lowercase()))
        });
    }
    if let Some(business_status) = query.business_status.as_deref() {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .is_some_and(|pi| pi.business_status.as_deref() == Some(business_status))
        });
    }
    if let Some(pattern) = query.business_status_like.as_deref() {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .and_then(|pi| pi.business_status.as_deref())
                .is_some_and(|status| sql_like_matches(pattern, status))
        });
    }
    if let Some(pattern) = query.business_status_like_ignore_case.as_deref() {
        let pattern = pattern.to_lowercase();
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .and_then(|pi| pi.business_status.as_deref())
                .is_some_and(|status| sql_like_matches(&pattern, &status.to_lowercase()))
        });
    }
    if let Some(callback_id) = query.callback_id.as_deref() {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .is_some_and(|pi| pi.callback_id.as_deref() == Some(callback_id))
        });
    }
    if let Some(callback_ids) = query.callback_ids.as_ref().filter(|ids| !ids.is_empty()) {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .and_then(|pi| pi.callback_id.clone())
                .is_some_and(|callback_id| callback_ids.contains(&callback_id))
        });
    }
    if let Some(callback_type) = query.callback_type.as_deref() {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .is_some_and(|pi| pi.callback_type.as_deref() == Some(callback_type))
        });
    }
    if query.without_callback_id == Some(true) {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .map_or(true, |pi| pi.callback_id.is_none())
        });
    }
    if let Some(root_scope_id) = query.root_scope_id.as_deref() {
        instances.retain(|instance| {
            runtime
                .get(&instance.id)
                .and_then(|pi| pi.root_process_instance_id.as_deref())
                .is_some_and(|root| root == root_scope_id)
        });
    }
    if let Some(state) = query.state.as_deref() {
        instances.retain(|instance| match state {
            "running" => {
                instance.end_time.is_none()
                    && !runtime.get(&instance.id).is_some_and(|pi| pi.is_suspended)
            }
            "suspended" => {
                instance.end_time.is_none()
                    && runtime.get(&instance.id).is_some_and(|pi| pi.is_suspended)
            }
            "completed" => instance.end_time.is_some() && instance.delete_reason.is_none(),
            "cancelled" => instance.end_time.is_some() && instance.delete_reason.is_some(),
            _ => false,
        });
    }

    if query.super_process_instance_id.is_some()
        || query.parent_scope_id.is_some()
        || query.exclude_subprocesses == Some(true)
    {
        let super_map = runtime_super_process_instance_map(&engine);
        if let Some(super_process_instance_id) = query.super_process_instance_id.as_deref() {
            instances.retain(|instance| {
                super_map.get(&instance.id).map(String::as_str) == Some(super_process_instance_id)
            });
        }
        if let Some(parent_scope_id) = query.parent_scope_id.as_deref() {
            instances.retain(|instance| {
                super_map.get(&instance.id).map(String::as_str) == Some(parent_scope_id)
            });
        }
        if query.exclude_subprocesses == Some(true) {
            instances.retain(|instance| !super_map.contains_key(&instance.id));
        }
    }

    if query.active_activity_id.is_some() || query.active_activity_ids.is_some() {
        let active: HashSet<(String, String)> = engine
            .get_history_service()
            .create_historic_activity_instance_query()
            .list()?
            .into_iter()
            .filter(|activity| activity.end_time.is_none())
            .map(|activity| (activity.process_instance_id, activity.activity_id))
            .collect();
        if let Some(activity_id) = query.active_activity_id.as_deref() {
            instances.retain(|instance| {
                active.contains(&(instance.id.clone(), activity_id.to_string()))
            });
        }
        if let Some(activity_ids) = query
            .active_activity_ids
            .as_ref()
            .filter(|ids| !ids.is_empty())
        {
            instances.retain(|instance| {
                activity_ids
                    .iter()
                    .any(|activity_id| active.contains(&(instance.id.clone(), activity_id.clone())))
            });
        }
    }

    if let Some(involved_user) = query.involved_user.as_deref() {
        // P77: Java HistoricProcessInstance.xml:903-904 uses ACT_HI_IDENTITYLINK.
        let involved: HashSet<String> = engine
            .get_history_service()
            .create_historic_identity_link_query()
            .user_id(involved_user.to_string())
            .list()
            .map_err(|error| ApiError::InternalServerError(error.to_string()))?
            .into_iter()
            .filter_map(|link| link.process_instance_id)
            .collect();
        instances.retain(|instance| involved.contains(&instance.id));
    }

    if let Some(filters) = query.variables.as_ref() {
        let all_variables = engine
            .get_history_service()
            .create_historic_variable_instance_query()
            .list()?;
        apply_historic_process_instance_variable_filters(&mut instances, &all_variables, filters)?;
    }

    sort_historic_process_instances(
        &mut instances,
        &definitions,
        query.sort.as_deref(),
        query.order.as_deref(),
    )?;
    Ok(instances)
}

fn paginate_historic_process_instances(
    engine: &ProcessEngine,
    query: HistoricProcessInstanceListQuery,
    instances: Vec<HistoricProcessInstance>,
) -> Result<PagedResponse<HistoricProcessInstanceResponse>, ApiError> {
    let definitions = historic_process_definition_meta(engine)?;
    let variable_names = query
        .include_process_variables_names
        .as_ref()
        .filter(|names| !names.is_empty());
    let include_process_variables =
        query.include_process_variables == Some(true) || variable_names.is_some();
    let mut result = Vec::new();
    for instance in instances {
        let mut response = to_historic_process_instance_response(instance, &definitions);
        if include_process_variables {
            let mut variables = historic_process_variable_instances(engine, &response.id)?;
            if let Some(names) = variable_names {
                variables.retain(|variable| names.contains(&variable.name));
            }
            response.variables = Some(historic_variable_response_scope(variables, "global"));
        }
        result.push(response);
    }

    Ok(query.paging().paginate(result))
}

#[derive(Debug, Clone)]
struct HistoricProcessDefinitionMeta {
    key: String,
    name: Option<String>,
    version: i32,
    category: Option<String>,
    deployment_id: Option<String>,
    tenant_id: Option<String>,
}

fn historic_process_definition_meta(
    engine: &ProcessEngine,
) -> Result<HashMap<String, HistoricProcessDefinitionMeta>, ApiError> {
    Ok(engine
        .get_repository_service()
        .get_process_definitions()?
        .into_iter()
        .map(|definition| {
            (
                definition.id,
                HistoricProcessDefinitionMeta {
                    key: definition.key,
                    name: definition.name,
                    version: definition.version,
                    category: definition.category,
                    deployment_id: definition.deployment_id,
                    tenant_id: definition.tenant_id,
                },
            )
        })
        .collect())
}

fn historic_process_definition_key(
    instance: &HistoricProcessInstance,
    definitions: &HashMap<String, HistoricProcessDefinitionMeta>,
) -> Option<String> {
    definitions
        .get(&instance.process_definition_id)
        .map(|definition| definition.key.clone())
        .or_else(|| {
            instance
                .process_definition_id
                .split_once(':')
                .map(|(key, _)| key.to_string())
        })
}

fn historic_process_tenant_id(
    instance: &HistoricProcessInstance,
    definitions: &HashMap<String, HistoricProcessDefinitionMeta>,
) -> Option<String> {
    definitions
        .get(&instance.process_definition_id)
        .and_then(|definition| definition.tenant_id.clone())
}

fn runtime_process_instances_by_id(
    engine: &ProcessEngine,
) -> HashMap<String, flowable_engine::runtime::process_instance::ProcessInstance> {
    engine
        .get_runtime_store()
        .db_store()
        .find_all::<flowable_engine::runtime::process_instance::ProcessInstance>(
            "process_instances",
        )
        .unwrap_or_default()
        .into_iter()
        .map(|instance| (instance.id.clone(), instance))
        .collect()
}

/// child process instance id → parent process instance id, resolved through
/// the call activity's super execution (runtime-only join: finished children
/// no longer carry the relationship).
fn runtime_super_process_instance_map(engine: &ProcessEngine) -> HashMap<String, String> {
    let executions = engine
        .get_runtime_store()
        .db_store()
        .find_all::<flowable_engine::runtime::execution::Execution>("executions")
        .unwrap_or_default();
    let execution_process_instance: HashMap<String, String> = executions
        .into_iter()
        .filter_map(|execution| {
            let process_instance_id = execution.process_instance_id?;
            Some((execution.id, process_instance_id))
        })
        .collect();
    runtime_process_instances_by_id(engine)
        .into_values()
        .filter_map(|instance| {
            let super_execution_id = instance.super_execution_id?;
            let parent = execution_process_instance.get(&super_execution_id)?;
            Some((instance.id, parent.clone()))
        })
        .collect()
}

fn sort_historic_process_instances(
    instances: &mut [HistoricProcessInstance],
    definitions: &HashMap<String, HistoricProcessDefinitionMeta>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    match sort {
        None | Some("processInstanceId") | Some("id") => {
            instances.sort_by(|left, right| left.id.cmp(&right.id))
        }
        Some("processDefinitionId") => instances.sort_by(|left, right| {
            left.process_definition_id
                .cmp(&right.process_definition_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("processDefinitionKey") => instances.sort_by(|left, right| {
            historic_process_definition_key(left, definitions)
                .cmp(&historic_process_definition_key(right, definitions))
                .then(left.id.cmp(&right.id))
        }),
        Some("businessKey") | Some("processInstanceBusinessKey") | Some("processBusinessKey") => {
            instances.sort_by(|left, right| {
                left.business_key
                    .cmp(&right.business_key)
                    .then(left.id.cmp(&right.id))
            })
        }
        Some("startTime") => instances.sort_by(|left, right| {
            left.start_time
                .cmp(&right.start_time)
                .then(left.id.cmp(&right.id))
        }),
        Some("endTime") => instances.sort_by(|left, right| {
            left.end_time
                .cmp(&right.end_time)
                .then(left.id.cmp(&right.id))
        }),
        Some("duration") | Some("durationInMillis") => instances.sort_by(|left, right| {
            left.duration_ms
                .cmp(&right.duration_ms)
                .then(left.id.cmp(&right.id))
        }),
        Some("tenantId") => instances.sort_by(|left, right| {
            historic_process_tenant_id(left, definitions)
                .cmp(&historic_process_tenant_id(right, definitions))
                .then(left.id.cmp(&right.id))
        }),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported historic process instance sort field '{other}'"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => instances.reverse(),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported historic process instance sort order '{other}'"
            )));
        }
    }

    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HistoricActivityInstanceListQuery {
    start: usize,
    size: Option<usize>,
    #[serde(rename = "activityInstanceId", alias = "id")]
    activity_instance_id: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "processInstanceIds")]
    process_instance_ids: Option<Vec<String>>,
    #[serde(rename = "processDefinitionId")]
    process_definition_id: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    #[serde(rename = "activityId")]
    activity_id: Option<String>,
    #[serde(rename = "activityName")]
    activity_name: Option<String>,
    #[serde(rename = "activityNameLike")]
    activity_name_like: Option<String>,
    #[serde(rename = "activityType")]
    activity_type: Option<String>,
    #[serde(rename = "taskAssignee")]
    task_assignee: Option<String>,
    #[serde(rename = "calledProcessInstanceIds")]
    called_process_instance_ids: Option<Vec<String>>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    #[serde(rename = "withoutTenantId")]
    without_tenant_id: Option<bool>,
    finished: Option<bool>,
    unfinished: Option<bool>,
    #[serde(rename = "startedAfter")]
    started_after: Option<String>,
    #[serde(rename = "startedBefore")]
    started_before: Option<String>,
    #[serde(rename = "finishedAfter")]
    finished_after: Option<String>,
    #[serde(rename = "finishedBefore")]
    finished_before: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl HistoricActivityInstanceListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HistoricTaskInstanceListQuery {
    start: usize,
    size: Option<usize>,
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "processInstanceIdWithChildren")]
    process_instance_id_with_children: Option<String>,
    #[serde(rename = "withoutProcessInstanceId")]
    without_process_instance_id: Option<bool>,
    #[serde(rename = "processBusinessKey")]
    process_business_key: Option<String>,
    #[serde(rename = "processBusinessKeyLike")]
    process_business_key_like: Option<String>,
    #[serde(rename = "processDefinitionId")]
    process_definition_id: Option<String>,
    #[serde(rename = "processDefinitionKey")]
    process_definition_key: Option<String>,
    #[serde(rename = "processDefinitionKeyLike")]
    process_definition_key_like: Option<String>,
    #[serde(rename = "processDefinitionName")]
    process_definition_name: Option<String>,
    #[serde(rename = "processDefinitionNameLike")]
    process_definition_name_like: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    #[serde(rename = "taskDefinitionKey")]
    task_definition_key: Option<String>,
    #[serde(rename = "taskDefinitionKeyLike")]
    task_definition_key_like: Option<String>,
    #[serde(rename = "taskDefinitionKeys")]
    task_definition_keys: Option<Vec<String>>,
    #[serde(rename = "taskName", alias = "name")]
    task_name: Option<String>,
    #[serde(rename = "taskNameLike", alias = "nameLike")]
    task_name_like: Option<String>,
    #[serde(rename = "taskNameLikeIgnoreCase")]
    task_name_like_ignore_case: Option<String>,
    #[serde(rename = "taskDescription")]
    task_description: Option<String>,
    #[serde(rename = "taskDescriptionLike")]
    task_description_like: Option<String>,
    #[serde(rename = "taskAssignee", alias = "assignee")]
    assignee: Option<String>,
    #[serde(rename = "taskAssigneeLike")]
    assignee_like: Option<String>,
    #[serde(rename = "taskOwner", alias = "owner")]
    owner: Option<String>,
    #[serde(rename = "taskOwnerLike")]
    owner_like: Option<String>,
    #[serde(rename = "taskInvolvedUser")]
    task_involved_user: Option<String>,
    #[serde(rename = "ignoreTaskAssignee")]
    ignore_task_assignee: Option<bool>,
    #[serde(rename = "taskCategory", alias = "category")]
    category: Option<String>,
    #[serde(rename = "taskCategoryIn")]
    task_category_in: Option<Vec<String>>,
    #[serde(rename = "taskCategoryNotIn")]
    task_category_not_in: Option<Vec<String>>,
    #[serde(rename = "taskWithoutCategory")]
    task_without_category: Option<bool>,
    #[serde(rename = "taskDeleteReason")]
    task_delete_reason: Option<String>,
    #[serde(rename = "taskDeleteReasonLike")]
    task_delete_reason_like: Option<String>,
    #[serde(rename = "withoutDeleteReason")]
    without_delete_reason: Option<bool>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    #[serde(rename = "withoutTenantId")]
    without_tenant_id: Option<bool>,
    #[serde(
        rename = "taskCandidateUser",
        alias = "candidateUser",
        alias = "candidateUsers"
    )]
    candidate_user: Option<String>,
    #[serde(
        rename = "taskCandidateGroup",
        alias = "candidateGroup",
        alias = "candidateGroups"
    )]
    candidate_group: Option<String>,
    #[serde(rename = "taskPriority", alias = "priority")]
    priority: Option<i32>,
    #[serde(rename = "taskMinPriority", alias = "minimumPriority")]
    minimum_priority: Option<i32>,
    #[serde(rename = "taskMaxPriority", alias = "maximumPriority")]
    maximum_priority: Option<i32>,
    #[serde(rename = "dueDate")]
    due_date: Option<String>,
    #[serde(rename = "dueDateBefore", alias = "dueBefore")]
    due_before: Option<String>,
    #[serde(rename = "dueDateAfter", alias = "dueAfter")]
    due_after: Option<String>,
    #[serde(rename = "withoutDueDate")]
    without_due_date: Option<bool>,
    finished: Option<bool>,
    unfinished: Option<bool>,
    #[serde(rename = "processFinished")]
    process_finished: Option<bool>,
    #[serde(rename = "taskCreatedOn")]
    task_created_on: Option<String>,
    #[serde(rename = "taskCreatedBefore")]
    task_created_before: Option<String>,
    #[serde(rename = "taskCreatedAfter")]
    task_created_after: Option<String>,
    #[serde(rename = "taskCompletedOn")]
    task_completed_on: Option<String>,
    #[serde(rename = "taskCompletedBefore")]
    task_completed_before: Option<String>,
    #[serde(rename = "taskCompletedAfter")]
    task_completed_after: Option<String>,
    #[serde(rename = "startedAfter")]
    started_after: Option<String>,
    #[serde(rename = "startedBefore")]
    started_before: Option<String>,
    #[serde(rename = "finishedAfter")]
    finished_after: Option<String>,
    #[serde(rename = "finishedBefore")]
    finished_before: Option<String>,
    #[serde(rename = "parentTaskId")]
    parent_task_id: Option<String>,
    #[serde(rename = "scopeDefinitionId")]
    scope_definition_id: Option<String>,
    #[serde(rename = "scopeId")]
    scope_id: Option<String>,
    #[serde(rename = "scopeIds")]
    scope_ids: Option<Vec<String>>,
    #[serde(rename = "withoutScopeId")]
    without_scope_id: Option<bool>,
    #[serde(rename = "scopeType")]
    scope_type: Option<String>,
    #[serde(rename = "propagatedStageInstanceId")]
    propagated_stage_instance_id: Option<String>,
    #[serde(rename = "rootScopeId")]
    root_scope_id: Option<String>,
    #[serde(rename = "parentScopeId")]
    parent_scope_id: Option<String>,
    #[serde(rename = "includeProcessVariables")]
    include_process_variables: Option<bool>,
    #[serde(rename = "includeTaskLocalVariables")]
    include_task_local_variables: Option<bool>,
    #[serde(rename = "taskVariables")]
    task_variables: Option<Vec<HistoricQueryVariable>>,
    #[serde(rename = "processVariables", alias = "processInstanceVariables")]
    process_instance_variables: Option<Vec<HistoricQueryVariable>>,
    sort: Option<String>,
    order: Option<String>,
}

impl HistoricTaskInstanceListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoricQueryVariable {
    name: Option<String>,
    operation: Option<String>,
    value: Option<Value>,
    /// Java `QueryVariable.type` (QueryVariable.java:66-71): accepted for JSON
    /// parity but not used for value conversion — matching is driven by the
    /// JSON value shape (P108 deviation, see query_variable.rs:21-27).
    #[serde(rename = "type")]
    _variable_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HistoricVariableListQuery {
    start: usize,
    size: Option<usize>,
    sort: Option<String>,
    order: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    #[serde(rename = "variableName", alias = "name")]
    variable_name: Option<String>,
    #[serde(rename = "variableNameLike")]
    variable_name_like: Option<String>,
    #[serde(rename = "variableType", alias = "type")]
    variable_type: Option<String>,
    #[serde(rename = "excludeTaskVariables")]
    exclude_task_variables: Option<bool>,
}

impl HistoricVariableListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HistoricDetailListQuery {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    #[serde(rename = "activityInstanceId")]
    activity_instance_id: Option<String>,
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    #[serde(rename = "type", alias = "detailType")]
    detail_type: Option<String>,
    #[serde(rename = "selectOnlyFormProperties")]
    select_only_form_properties: bool,
    #[serde(rename = "selectOnlyVariableUpdates")]
    select_only_variable_updates: bool,
    #[serde(rename = "variableName")]
    variable_name: Option<String>,
    #[serde(rename = "formPropertyId", alias = "propertyId")]
    property_id: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HistoricTaskLogEntryListQuery {
    start: usize,
    size: Option<usize>,
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    #[serde(rename = "type")]
    log_type: Option<String>,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "processDefinitionId")]
    process_definition_id: Option<String>,
    #[serde(rename = "scopeId")]
    scope_id: Option<String>,
    #[serde(rename = "scopeDefinitionId")]
    scope_definition_id: Option<String>,
    #[serde(rename = "subScopeId")]
    sub_scope_id: Option<String>,
    #[serde(rename = "scopeType")]
    scope_type: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    #[serde(rename = "fromLogNumber")]
    from_log_number: Option<i64>,
    #[serde(rename = "toLogNumber")]
    to_log_number: Option<i64>,
    sort: Option<String>,
    order: Option<String>,
}

impl HistoricTaskLogEntryListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

impl HistoricDetailListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricDetailResponse {
    id: String,
    process_instance_id: String,
    process_instance_url: String,
    execution_id: Option<String>,
    activity_instance_id: Option<String>,
    task_id: Option<String>,
    task_url: Option<String>,
    time: String,
    detail_type: String,
    revision: Option<i32>,
    variable: Option<HistoricDetailVariableResponse>,
    property_id: Option<String>,
    property_value: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricDetailVariableResponse {
    name: String,
    #[serde(rename = "type")]
    variable_type: String,
    value: Value,
    value_url: Option<String>,
}

fn to_historic_detail_response(detail: HistoricDetail) -> HistoricDetailResponse {
    let variable = match (
        detail.variable_name.clone(),
        detail.variable_type.clone(),
        detail.value.clone(),
    ) {
        (Some(name), Some(variable_type), Some(value)) => {
            let variable_type = persisted_or_inferred_variable_type(&variable_type, &value);
            Some(HistoricDetailVariableResponse {
                name,
                variable_type,
                value,
                value_url: None,
            })
        }
        _ => None,
    };

    HistoricDetailResponse {
        process_instance_url: format!(
            "/history/historic-process-instances/{}",
            detail.process_instance_id
        ),
        task_url: detail
            .task_id
            .as_ref()
            .map(|task_id| format!("/history/historic-task-instances/{task_id}")),
        id: detail.id,
        process_instance_id: detail.process_instance_id,
        execution_id: detail.execution_id,
        activity_instance_id: detail.activity_instance_id,
        task_id: detail.task_id,
        time: detail.time.to_rfc3339(),
        detail_type: detail.detail_type,
        revision: detail.revision,
        variable,
        property_id: detail.property_id,
        property_value: detail.property_value,
    }
}

pub(crate) async fn historic_details(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricDetailResponse>>, ApiError> {
    let query: HistoricDetailListQuery = parse_query(&uri)?;
    Ok(Json(historic_details_for_query(engine, &query)))
}

pub(crate) async fn query_historic_details(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<HistoricDetailResponse>>, ApiError> {
    let mut query: HistoricDetailListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: HistoricDetailListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);

    Ok(Json(historic_details_for_query(engine, &query)))
}

fn historic_details_for_query(
    engine: Arc<ProcessEngine>,
    query: &HistoricDetailListQuery,
) -> PagedResponse<HistoricDetailResponse> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let mut details = engine
        .get_history_service()
        .get_historic_details(&mut session);

    if let Some(id) = query.id.as_deref() {
        details.retain(|detail| detail.id == id);
    }
    if let Some(process_instance_id) = query.process_instance_id.as_deref() {
        details.retain(|detail| detail.process_instance_id == process_instance_id);
    }
    if let Some(execution_id) = query.execution_id.as_deref() {
        details.retain(|detail| detail.execution_id.as_deref() == Some(execution_id));
    }
    if let Some(activity_instance_id) = query.activity_instance_id.as_deref() {
        details
            .retain(|detail| detail.activity_instance_id.as_deref() == Some(activity_instance_id));
    }
    if let Some(task_id) = query.task_id.as_deref() {
        details.retain(|detail| detail.task_id.as_deref() == Some(task_id));
    }
    if query.select_only_form_properties {
        details.retain(|detail| detail.detail_type == "formProperty");
    }
    if query.select_only_variable_updates {
        details.retain(|detail| detail.detail_type == "variableUpdate");
    }
    if let Some(detail_type) = query.detail_type.as_deref() {
        details.retain(|detail| detail.detail_type == detail_type);
    }
    if let Some(variable_name) = query.variable_name.as_deref() {
        details.retain(|detail| detail.variable_name.as_deref() == Some(variable_name));
    }
    if let Some(property_id) = query.property_id.as_deref() {
        details.retain(|detail| detail.property_id.as_deref() == Some(property_id));
    }
    sort_historic_details(&mut details, query.sort.as_deref(), query.order.as_deref());

    let result = details
        .into_iter()
        .map(to_historic_detail_response)
        .collect();
    query.paging().paginate(result)
}

fn sort_historic_details(details: &mut [HistoricDetail], sort: Option<&str>, order: Option<&str>) {
    match sort.unwrap_or("processInstanceId") {
        "time" => {
            details.sort_by(|left, right| left.time.cmp(&right.time).then(left.id.cmp(&right.id)))
        }
        "name" => details.sort_by(|left, right| {
            left.variable_name
                .cmp(&right.variable_name)
                .then(left.property_id.cmp(&right.property_id))
                .then(left.id.cmp(&right.id))
        }),
        "revision" => details.sort_by(|left, right| {
            left.revision
                .cmp(&right.revision)
                .then(left.id.cmp(&right.id))
        }),
        "variableType" => details.sort_by(|left, right| {
            left.variable_type
                .cmp(&right.variable_type)
                .then(left.id.cmp(&right.id))
        }),
        _ => details.sort_by(|left, right| {
            left.process_instance_id
                .cmp(&right.process_instance_id)
                .then(left.time.cmp(&right.time))
                .then(left.id.cmp(&right.id))
        }),
    }
    if matches!(order, Some(value) if value.eq_ignore_ascii_case("desc")) {
        details.reverse();
    }
}

pub(crate) async fn get_historic_detail_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(detail_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let detail = engine
        .get_history_service()
        .get_historic_detail(&detail_id, &mut session)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Historic detail '{}' was not found", detail_id))
        })?;
    let variable_type = detail.variable_type.as_deref().unwrap_or_default();
    if !matches!(variable_type, "binary" | "bytes" | "serializable") {
        return Err(ApiError::NotFound(format!(
            "Historic detail '{}' does not have a binary data stream",
            detail.id
        )));
    }
    let data = detail.value.ok_or_else(|| {
        ApiError::NotFound(format!(
            "Historic detail '{}' does not contain binary or JSON data",
            detail.id
        ))
    })?;

    Ok(Json(data))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricTaskLogEntryResponse {
    log_number: i64,
    #[serde(rename = "type")]
    log_type: String,
    task_id: String,
    time_stamp: String,
    user_id: Option<String>,
    data: Option<String>,
    execution_id: Option<String>,
    process_instance_id: Option<String>,
    process_definition_id: Option<String>,
    scope_id: Option<String>,
    scope_definition_id: Option<String>,
    sub_scope_id: Option<String>,
    scope_type: Option<String>,
    tenant_id: Option<String>,
}

fn to_historic_task_log_entry_response(
    entry: HistoricTaskLogEntry,
) -> HistoricTaskLogEntryResponse {
    HistoricTaskLogEntryResponse {
        log_number: entry.log_number,
        log_type: entry.log_type,
        task_id: entry.task_id,
        time_stamp: entry.timestamp.to_rfc3339(),
        user_id: entry.user_id,
        data: entry.data,
        execution_id: entry.execution_id,
        process_instance_id: entry.process_instance_id,
        process_definition_id: entry.process_definition_id,
        scope_id: entry.scope_id,
        scope_definition_id: entry.scope_definition_id,
        sub_scope_id: entry.sub_scope_id,
        scope_type: entry.scope_type,
        tenant_id: entry.tenant_id,
    }
}

pub(crate) async fn historic_task_log_entries(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricTaskLogEntryResponse>>, ApiError> {
    let query: HistoricTaskLogEntryListQuery = parse_query(&uri)?;
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let mut entries = engine
        .get_history_service()
        .get_historic_task_log_entries(&mut session);

    if let Some(task_id) = query.task_id.as_deref() {
        entries.retain(|entry| entry.task_id == task_id);
    }
    if let Some(log_type) = query.log_type.as_deref() {
        entries.retain(|entry| entry.log_type == log_type);
    }
    if let Some(user_id) = query.user_id.as_deref() {
        entries.retain(|entry| entry.user_id.as_deref() == Some(user_id));
    }
    if let Some(process_instance_id) = query.process_instance_id.as_deref() {
        entries.retain(|entry| entry.process_instance_id.as_deref() == Some(process_instance_id));
    }
    if let Some(process_definition_id) = query.process_definition_id.as_deref() {
        entries
            .retain(|entry| entry.process_definition_id.as_deref() == Some(process_definition_id));
    }
    if let Some(scope_id) = query.scope_id.as_deref() {
        entries.retain(|entry| entry.scope_id.as_deref() == Some(scope_id));
    }
    if let Some(scope_definition_id) = query.scope_definition_id.as_deref() {
        entries.retain(|entry| entry.scope_definition_id.as_deref() == Some(scope_definition_id));
    }
    if let Some(sub_scope_id) = query.sub_scope_id.as_deref() {
        entries.retain(|entry| entry.sub_scope_id.as_deref() == Some(sub_scope_id));
    }
    if let Some(scope_type) = query.scope_type.as_deref() {
        entries.retain(|entry| entry.scope_type.as_deref() == Some(scope_type));
    }
    if let Some(tenant_id) = query.tenant_id.as_deref() {
        entries.retain(|entry| entry.tenant_id.as_deref() == Some(tenant_id));
    }
    if let Some(from_log_number) = query.from_log_number {
        entries.retain(|entry| entry.log_number >= from_log_number);
    }
    if let Some(to_log_number) = query.to_log_number {
        entries.retain(|entry| entry.log_number <= to_log_number);
    }
    if let Some(from) = query.from.as_deref() {
        let from = parse_timestamp_millis(from)?;
        entries.retain(|entry| entry.timestamp.timestamp_millis() >= from);
    }
    if let Some(to) = query.to.as_deref() {
        let to = parse_timestamp_millis(to)?;
        entries.retain(|entry| entry.timestamp.timestamp_millis() <= to);
    }
    sort_historic_task_log_entries(&mut entries, query.sort.as_deref(), query.order.as_deref());

    let result = entries
        .into_iter()
        .map(to_historic_task_log_entry_response)
        .collect();
    Ok(Json(query.paging().paginate(result)))
}

fn parse_timestamp_millis(value: &str) -> Result<i64, ApiError> {
    if let Ok(timestamp) = value.parse::<i64>() {
        return Ok(timestamp);
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .map_err(|error| ApiError::BadRequest(format!("Invalid timestamp '{}': {}", value, error)))
}

fn sort_historic_task_log_entries(
    entries: &mut [HistoricTaskLogEntry],
    sort: Option<&str>,
    order: Option<&str>,
) {
    match sort.unwrap_or("logNumber") {
        "timeStamp" | "timestamp" => entries.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then(left.log_number.cmp(&right.log_number))
        }),
        "taskId" => entries.sort_by(|left, right| {
            left.task_id
                .cmp(&right.task_id)
                .then(left.log_number.cmp(&right.log_number))
        }),
        "type" => entries.sort_by(|left, right| {
            left.log_type
                .cmp(&right.log_type)
                .then(left.log_number.cmp(&right.log_number))
        }),
        _ => entries.sort_by_key(|left| left.log_number),
    }
    if matches!(order, Some(value) if value.eq_ignore_ascii_case("desc")) {
        entries.reverse();
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricTaskInstanceResponse {
    id: String,
    process_instance_id: String,
    process_definition_id: Option<String>,
    execution_id: String,
    task_definition_key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    assignee: Option<String>,
    owner: Option<String>,
    claim_time: Option<String>,
    form_key: Option<String>,
    tenant_id: Option<String>,
    parent_task_id: Option<String>,
    category: Option<String>,
    candidate_users: Vec<String>,
    candidate_groups: Vec<String>,
    priority: Option<i32>,
    due_date: Option<String>,
    start_time: String,
    end_time: Option<String>,
    duration_in_millis: Option<i64>,
    work_time_in_millis: Option<i64>,
    delete_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variables: Option<Vec<super::process_instances::RestVariableResponse>>,
}

fn to_historic_task_instance_response(
    engine: &ProcessEngine,
    task: HistoricTaskInstance,
) -> HistoricTaskInstanceResponse {
    let (candidate_users, candidate_groups) =
        super::tasks::candidate_identity_ids(engine, &task.id);
    // Java HistoricTaskInstanceEntityImpl#getWorkTimeInMillis: work starts when
    // the task is claimed and ends when the task finishes.
    let work_time_in_millis = task
        .end_time
        .zip(task.claim_time)
        .map(|(end, claim)| end.signed_duration_since(claim).num_milliseconds());
    HistoricTaskInstanceResponse {
        id: task.id,
        process_instance_id: task.process_instance_id,
        process_definition_id: task.process_definition_id,
        execution_id: task.execution_id,
        task_definition_key: task.task_definition_key,
        name: task.name,
        description: task.description,
        assignee: task.assignee,
        owner: task.owner,
        claim_time: task.claim_time.map(|claim_time| claim_time.to_rfc3339()),
        form_key: task.form_key,
        tenant_id: task.tenant_id,
        parent_task_id: task.parent_task_id,
        category: task.category,
        candidate_users,
        candidate_groups,
        priority: task.priority,
        due_date: task.due_date.map(|due_date| due_date.to_rfc3339()),
        start_time: task.start_time.to_rfc3339(),
        end_time: task.end_time.map(|time| time.to_rfc3339()),
        duration_in_millis: task.duration_ms,
        work_time_in_millis,
        delete_reason: task.delete_reason,
        variables: None,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricIdentityLinkResponse {
    #[serde(rename = "type")]
    link_type: String,
    user_id: Option<String>,
    group_id: Option<String>,
    task_id: Option<String>,
    task_url: Option<String>,
    process_instance_id: Option<String>,
    process_instance_url: Option<String>,
}

impl From<IdentityLink> for HistoricIdentityLinkResponse {
    fn from(link: IdentityLink) -> Self {
        let task_url = link
            .task_id
            .as_ref()
            .map(|task_id| format!("/history/historic-task-instances/{task_id}"));
        let process_instance_url = link
            .process_instance_id
            .as_ref()
            .map(|process_instance_id| {
                format!("/history/historic-process-instances/{process_instance_id}")
            });

        Self {
            link_type: link.link_type,
            user_id: link.user_id,
            group_id: link.group_id,
            task_id: link.task_id,
            task_url,
            process_instance_id: link.process_instance_id,
            process_instance_url,
        }
    }
}

impl From<HistoricIdentityLink> for HistoricIdentityLinkResponse {
    fn from(link: HistoricIdentityLink) -> Self {
        let task_url = link
            .task_id
            .as_ref()
            .map(|task_id| format!("/history/historic-task-instances/{task_id}"));
        let process_instance_url = link
            .process_instance_id
            .as_ref()
            .map(|process_instance_id| {
                format!("/history/historic-process-instances/{process_instance_id}")
            });

        Self {
            link_type: link.link_type,
            user_id: link.user_id,
            group_id: link.group_id,
            task_id: link.task_id,
            task_url,
            process_instance_id: link.process_instance_id,
            process_instance_url,
        }
    }
}

pub(crate) async fn query_historic_task_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<HistoricTaskInstanceResponse>>, ApiError> {
    let mut query: HistoricTaskInstanceListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: HistoricTaskInstanceListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);
    query.include_process_variables = url_query
        .include_process_variables
        .or(query.include_process_variables);
    query.include_task_local_variables = url_query
        .include_task_local_variables
        .or(query.include_task_local_variables);

    Ok(Json(historic_task_instances_for_query(engine, query)?))
}

pub(crate) async fn historic_task_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricTaskInstanceResponse>>, ApiError> {
    let query: HistoricTaskInstanceListQuery = parse_query(&uri)?;
    Ok(Json(historic_task_instances_for_query(engine, query)?))
}

pub(crate) async fn get_historic_task_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<Json<HistoricTaskInstanceResponse>, ApiError> {
    let task = load_historic_task_instance(&engine, &task_id)?;

    Ok(Json(to_historic_task_instance_response(&engine, task)))
}

pub(crate) async fn delete_historic_task_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_history_service()
        .delete_historic_task_instance(task_id)?;
    Ok(StatusCode::NO_CONTENT)
}

fn load_historic_task_instance(
    engine: &ProcessEngine,
    task_id: &str,
) -> Result<HistoricTaskInstance, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_runtime_store()
        .get_historic_task_instance(task_id, &mut session)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic task instance '{}' was not found",
                task_id
            ))
        })
}

fn ensure_historic_task_instance_exists(
    engine: &ProcessEngine,
    task_id: &str,
) -> Result<(), ApiError> {
    engine
        .get_history_service()
        .create_historic_task_instance_query()
        .list()?
        .into_iter()
        .find(|task| task.id == task_id)
        .map(|_| ())
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic task instance '{}' was not found",
                task_id
            ))
        })
}

pub(crate) async fn get_historic_task_instance_identity_links(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<HistoricIdentityLinkResponse>>, ApiError> {
    ensure_historic_task_instance_exists(&engine, &task_id)?;
    // P77: Java HistoryService.getHistoricIdentityLinksForTask reads
    // ACT_HI_IDENTITYLINK (historic_identity_links).
    let links = engine
        .get_history_service()
        .get_historic_identity_links_for_task(&task_id)
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?
        .into_iter()
        .map(HistoricIdentityLinkResponse::from)
        .collect();

    Ok(Json(links))
}

pub(crate) async fn get_historic_task_instance_form(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<Json<HistoricTaskFormResponse>, ApiError> {
    let _task = load_historic_task_instance(&engine, &task_id)?;
    let mut instances = FlowableFormService::new(engine)
        .create_form_instance_query()
        .task_id(task_id.clone())
        .list()?;
    let instance = instances.pop().ok_or_else(|| {
        ApiError::NotFound(format!(
            "Historic form data for task '{}' was not found",
            task_id
        ))
    })?;

    Ok(Json(historic_task_form_response(instance)))
}

fn historic_task_form_response(instance: FormInstance) -> HistoricTaskFormResponse {
    HistoricTaskFormResponse {
        id: instance.id,
        form_definition_id: instance.form_definition_id,
        form_definition_key: instance.form_definition_key,
        form_definition_name: instance.form_definition_name,
        deployment_id: instance.deployment_id,
        process_definition_id: instance.process_definition_id,
        process_instance_id: instance.process_instance_id,
        task_id: instance.task_id,
        submitted_at: instance.submitted_at,
        values: instance.values,
    }
}

fn historic_task_instances_for_query(
    engine: Arc<ProcessEngine>,
    query: HistoricTaskInstanceListQuery,
) -> Result<PagedResponse<HistoricTaskInstanceResponse>, ApiError> {
    let mut task_query = engine
        .get_history_service()
        .create_historic_task_instance_query();
    if let Some(process_instance_id) = query.process_instance_id.clone() {
        task_query = task_query.process_instance_id(process_instance_id);
    }
    if let Some(process_definition_id) = query.process_definition_id.clone() {
        task_query = task_query.process_definition_id(process_definition_id);
    }
    if let Some(task_definition_key) = query.task_definition_key.clone() {
        task_query = task_query.task_definition_key(task_definition_key);
    }
    if let Some(task_definition_key_like) = query.task_definition_key_like.clone() {
        task_query = task_query.task_definition_key_like(task_definition_key_like);
    }
    if let Some(assignee) = query.assignee.clone() {
        task_query = task_query.task_assignee(assignee);
    }
    if let Some(owner) = query.owner.clone() {
        task_query = task_query.task_owner(owner);
    }
    if let Some(candidate_user) = query.candidate_user.clone() {
        task_query = task_query.task_candidate_user(candidate_user);
    }
    if let Some(candidate_group) = query.candidate_group.clone() {
        task_query = task_query.task_candidate_group(candidate_group);
    }
    // Java HistoricTaskInstanceBaseResource: ignoreTaskAssignee → ignoreAssigneeValue
    // (HistoricTaskInstanceBaseResource.java:294-296). Widens candidate filters to
    // include assigned historic tasks (HistoricTaskInstance.xml:1485-1487).
    if query.ignore_task_assignee == Some(true) {
        task_query = task_query.ignore_assignee_value();
    }
    if let Some(priority) = query.priority {
        task_query = task_query.task_priority(priority);
    }
    if let Some(minimum_priority) = query.minimum_priority {
        task_query = task_query.task_minimum_priority(minimum_priority);
    }
    if let Some(maximum_priority) = query.maximum_priority {
        task_query = task_query.task_maximum_priority(maximum_priority);
    }
    if let Some(due_date) = query.due_date.as_deref() {
        task_query = task_query.task_due_date_millis(parse_timestamp_millis(due_date)?);
    }
    if let Some(due_before) = query.due_before.as_deref() {
        task_query = task_query.task_due_before_millis(parse_timestamp_millis(due_before)?);
    }
    if let Some(due_after) = query.due_after.as_deref() {
        task_query = task_query.task_due_after_millis(parse_timestamp_millis(due_after)?);
    }
    if query.without_due_date == Some(true) {
        task_query = task_query.task_without_due_date();
    }
    let mut tasks = task_query.list()?;
    if let Some(task_id) = query.task_id.as_deref() {
        tasks.retain(|task| task.id == task_id);
    }
    if let Some(task_name) = query.task_name.as_deref() {
        tasks.retain(|task| task.name.as_deref() == Some(task_name));
    }
    if let Some(task_name_like) = query.task_name_like.as_deref() {
        tasks.retain(|task| {
            task.name
                .as_deref()
                .is_some_and(|name| sql_like_matches(task_name_like, name))
        });
    }
    if let Some(category) = query.category.as_deref() {
        tasks.retain(|task| historic_task_category(task).as_deref() == Some(category));
    }
    if let Some(tenant_id) = query.tenant_id.as_deref() {
        tasks.retain(|task| historic_task_tenant_id(task).as_deref() == Some(tenant_id));
    }
    if let Some(finished) = query.finished {
        tasks.retain(|task| task.end_time.is_some() == finished);
    }
    if query.unfinished == Some(true) {
        tasks.retain(|task| task.end_time.is_none());
    }
    if let Some(started_after) = query.started_after.as_deref() {
        let started_after = parse_timestamp_millis(started_after)?;
        tasks.retain(|task| task.start_time.timestamp_millis() >= started_after);
    }
    if let Some(started_before) = query.started_before.as_deref() {
        let started_before = parse_timestamp_millis(started_before)?;
        tasks.retain(|task| task.start_time.timestamp_millis() <= started_before);
    }
    if let Some(finished_after) = query.finished_after.as_deref() {
        let finished_after = parse_timestamp_millis(finished_after)?;
        tasks.retain(|task| {
            task.end_time
                .is_some_and(|end_time| end_time.timestamp_millis() >= finished_after)
        });
    }
    if let Some(finished_before) = query.finished_before.as_deref() {
        let finished_before = parse_timestamp_millis(finished_before)?;
        tasks.retain(|task| {
            task.end_time
                .is_some_and(|end_time| end_time.timestamp_millis() <= finished_before)
        });
    }
    if let Some(assignee_like) = query.assignee_like.as_deref() {
        tasks.retain(|task| {
            task.assignee
                .as_deref()
                .is_some_and(|assignee| sql_like_matches(assignee_like, assignee))
        });
    }
    if let Some(owner_like) = query.owner_like.as_deref() {
        tasks.retain(|task| {
            task.owner
                .as_deref()
                .is_some_and(|owner| sql_like_matches(owner_like, owner))
        });
    }
    if let Some(pattern) = query.task_name_like_ignore_case.as_deref() {
        let pattern = pattern.to_lowercase();
        tasks.retain(|task| {
            task.name
                .as_deref()
                .is_some_and(|name| sql_like_matches(&pattern, &name.to_lowercase()))
        });
    }
    if let Some(description) = query.task_description.as_deref() {
        tasks.retain(|task| task.description.as_deref() == Some(description));
    }
    if let Some(pattern) = query.task_description_like.as_deref() {
        tasks.retain(|task| {
            task.description
                .as_deref()
                .is_some_and(|description| sql_like_matches(pattern, description))
        });
    }
    if let Some(keys) = query
        .task_definition_keys
        .as_ref()
        .filter(|keys| !keys.is_empty())
    {
        tasks.retain(|task| {
            task.task_definition_key
                .as_ref()
                .is_some_and(|key| keys.contains(key))
        });
    }
    if let Some(execution_id) = query.execution_id.as_deref() {
        tasks.retain(|task| task.execution_id == execution_id);
    }
    if query.without_process_instance_id == Some(true) {
        tasks.retain(|task| task.process_instance_id.is_empty());
    }
    if let Some(delete_reason) = query.task_delete_reason.as_deref() {
        tasks.retain(|task| task.delete_reason.as_deref() == Some(delete_reason));
    }
    if let Some(pattern) = query.task_delete_reason_like.as_deref() {
        tasks.retain(|task| {
            task.delete_reason
                .as_deref()
                .is_some_and(|reason| sql_like_matches(pattern, reason))
        });
    }
    if query.without_delete_reason == Some(true) {
        tasks.retain(|task| task.delete_reason.is_none());
    }
    if let Some(created_on) = query.task_created_on.as_deref() {
        let created_on = parse_timestamp_millis(created_on)?;
        tasks.retain(|task| task.start_time.timestamp_millis() == created_on);
    }
    if let Some(created_before) = query.task_created_before.as_deref() {
        let created_before = parse_timestamp_millis(created_before)?;
        tasks.retain(|task| task.start_time.timestamp_millis() <= created_before);
    }
    if let Some(created_after) = query.task_created_after.as_deref() {
        let created_after = parse_timestamp_millis(created_after)?;
        tasks.retain(|task| task.start_time.timestamp_millis() >= created_after);
    }
    if let Some(completed_on) = query.task_completed_on.as_deref() {
        let completed_on = parse_timestamp_millis(completed_on)?;
        tasks.retain(|task| {
            task.end_time
                .is_some_and(|end_time| end_time.timestamp_millis() == completed_on)
        });
    }
    if let Some(completed_before) = query.task_completed_before.as_deref() {
        let completed_before = parse_timestamp_millis(completed_before)?;
        tasks.retain(|task| {
            task.end_time
                .is_some_and(|end_time| end_time.timestamp_millis() <= completed_before)
        });
    }
    if let Some(completed_after) = query.task_completed_after.as_deref() {
        let completed_after = parse_timestamp_millis(completed_after)?;
        tasks.retain(|task| {
            task.end_time
                .is_some_and(|end_time| end_time.timestamp_millis() >= completed_after)
        });
    }
    if query.process_finished.is_some()
        || query.process_business_key.is_some()
        || query.process_business_key_like.is_some()
    {
        let process_instances: HashMap<String, HistoricProcessInstance> = engine
            .get_history_service()
            .create_historic_process_instance_query()
            .list()?
            .into_iter()
            .map(|instance| (instance.id.clone(), instance))
            .collect();
        if let Some(process_finished) = query.process_finished {
            tasks.retain(|task| {
                process_instances
                    .get(&task.process_instance_id)
                    .and_then(|instance| instance.end_time)
                    .is_some()
                    == process_finished
            });
        }
        if let Some(business_key) = query.process_business_key.as_deref() {
            tasks.retain(|task| {
                process_instances
                    .get(&task.process_instance_id)
                    .and_then(|instance| instance.business_key.as_deref())
                    == Some(business_key)
            });
        }
        if let Some(pattern) = query.process_business_key_like.as_deref() {
            tasks.retain(|task| {
                process_instances
                    .get(&task.process_instance_id)
                    .and_then(|instance| instance.business_key.as_deref())
                    .is_some_and(|business_key| sql_like_matches(pattern, business_key))
            });
        }
    }
    if query.process_definition_key.is_some()
        || query.process_definition_key_like.is_some()
        || query.process_definition_name.is_some()
        || query.process_definition_name_like.is_some()
    {
        let definitions = historic_process_definition_meta(&engine)?;
        if let Some(key) = query.process_definition_key.as_deref() {
            tasks.retain(|task| {
                task.process_definition_id
                    .as_deref()
                    .and_then(|id| definitions.get(id))
                    .is_some_and(|definition| definition.key == key)
            });
        }
        if let Some(pattern) = query.process_definition_key_like.as_deref() {
            tasks.retain(|task| {
                task.process_definition_id
                    .as_deref()
                    .and_then(|id| definitions.get(id))
                    .is_some_and(|definition| sql_like_matches(pattern, &definition.key))
            });
        }
        if let Some(name) = query.process_definition_name.as_deref() {
            tasks.retain(|task| {
                task.process_definition_id
                    .as_deref()
                    .and_then(|id| definitions.get(id))
                    .is_some_and(|definition| definition.name.as_deref() == Some(name))
            });
        }
        if let Some(pattern) = query.process_definition_name_like.as_deref() {
            tasks.retain(|task| {
                task.process_definition_id
                    .as_deref()
                    .and_then(|id| definitions.get(id))
                    .and_then(|definition| definition.name.as_deref())
                    .is_some_and(|name| sql_like_matches(pattern, name))
            });
        }
    }
    if let Some(root_id) = query.process_instance_id_with_children.as_deref() {
        // Runtime-only join: resolve the call activity parent chain upwards.
        let super_map = runtime_super_process_instance_map(&engine);
        tasks.retain(|task| {
            let mut current = task.process_instance_id.clone();
            loop {
                if current == root_id {
                    return true;
                }
                match super_map.get(&current) {
                    Some(parent) => current = parent.clone(),
                    None => return false,
                }
            }
        });
    }
    if let Some(involved_user) = query.task_involved_user.as_deref() {
        let involved: HashSet<String> = engine
            .get_identity_link_service()
            .create_identity_link_query()
            .user_id(involved_user.to_string())
            .list()
            .map_err(|error| ApiError::InternalServerError(error.to_string()))?
            .into_iter()
            .filter_map(|link| link.task_id)
            .collect();
        tasks.retain(|task| involved.contains(&task.id));
    }
    // ignoreTaskAssignee is applied on the query builder above
    // (HistoricTaskInstanceBaseResource.java:294-296 → ignoreAssigneeValue).
    if let Some(categories) = query
        .task_category_in
        .as_ref()
        .filter(|categories| !categories.is_empty())
    {
        tasks.retain(|task| {
            historic_task_category(task).is_some_and(|category| categories.contains(&category))
        });
    }
    if let Some(categories) = query
        .task_category_not_in
        .as_ref()
        .filter(|categories| !categories.is_empty())
    {
        // SQL NOT IN semantics: a NULL category never matches the predicate.
        tasks.retain(|task| {
            historic_task_category(task).is_some_and(|category| !categories.contains(&category))
        });
    }
    if query.task_without_category == Some(true) {
        tasks.retain(|task| historic_task_category(task).is_none());
    }
    if let Some(pattern) = query.tenant_id_like.as_deref() {
        tasks.retain(|task| {
            historic_task_tenant_id(task)
                .is_some_and(|tenant_id| sql_like_matches(pattern, &tenant_id))
        });
    }
    if query.without_tenant_id == Some(true) {
        tasks.retain(|task| historic_task_tenant_id(task).is_none());
    }
    // Scope / parent task columns are not persisted for historic BPMN tasks
    // (Java stores NULL): equality filters are honest empty matches while
    // withoutScopeId keeps every row.
    if let Some(scope_id) = query.scope_id.as_deref() {
        tasks.retain(|task| {
            historic_task_string_field(task, "scopeId", "scope_id").as_deref() == Some(scope_id)
        });
    }
    if let Some(scope_ids) = query.scope_ids.as_ref().filter(|ids| !ids.is_empty()) {
        tasks.retain(|task| {
            historic_task_string_field(task, "scopeId", "scope_id")
                .is_some_and(|id| scope_ids.contains(&id))
        });
    }
    if query.without_scope_id == Some(true) {
        tasks.retain(|task| historic_task_string_field(task, "scopeId", "scope_id").is_none());
    }
    if let Some(scope_type) = query.scope_type.as_deref() {
        tasks.retain(|task| {
            historic_task_string_field(task, "scopeType", "scope_type").as_deref()
                == Some(scope_type)
        });
    }
    if let Some(scope_definition_id) = query.scope_definition_id.as_deref() {
        tasks.retain(|task| {
            historic_task_string_field(task, "scopeDefinitionId", "scope_definition_id").as_deref()
                == Some(scope_definition_id)
        });
    }
    if let Some(stage_instance_id) = query.propagated_stage_instance_id.as_deref() {
        tasks.retain(|task| {
            historic_task_string_field(
                task,
                "propagatedStageInstanceId",
                "propagated_stage_instance_id",
            )
            .as_deref()
                == Some(stage_instance_id)
        });
    }
    if let Some(parent_task_id) = query.parent_task_id.as_deref() {
        tasks.retain(|task| {
            historic_task_string_field(task, "parentTaskId", "parent_task_id").as_deref()
                == Some(parent_task_id)
        });
    }
    if let Some(root_scope_id) = query.root_scope_id.as_deref() {
        tasks.retain(|task| {
            historic_task_string_field(task, "rootScopeId", "root_scope_id").as_deref()
                == Some(root_scope_id)
        });
    }
    if let Some(parent_scope_id) = query.parent_scope_id.as_deref() {
        tasks.retain(|task| {
            historic_task_string_field(task, "parentScopeId", "parent_scope_id").as_deref()
                == Some(parent_scope_id)
        });
    }
    if let Some(task_variables) = query.task_variables.as_ref() {
        let variables = engine
            .get_history_service()
            .create_historic_variable_instance_query()
            .list()?;
        apply_historic_task_variable_filters(
            &mut tasks,
            &variables,
            task_variables,
            HistoricTaskVariableScope::TaskLocal,
        )?;
    }
    if let Some(process_instance_variables) = query.process_instance_variables.as_ref() {
        let variables = engine
            .get_history_service()
            .create_historic_variable_instance_query()
            .list()?;
        apply_historic_task_variable_filters(
            &mut tasks,
            &variables,
            process_instance_variables,
            HistoricTaskVariableScope::ProcessInstance,
        )?;
    }
    sort_historic_task_instances(&mut tasks, query.sort.as_deref(), query.order.as_deref())?;

    let include_process_variables = query.include_process_variables == Some(true);
    let include_task_local_variables = query.include_task_local_variables == Some(true);
    let mut result = Vec::new();
    for task in tasks {
        let variables = if include_process_variables || include_task_local_variables {
            Some(historic_task_variable_responses(
                &engine,
                &task,
                include_process_variables,
                include_task_local_variables,
            )?)
        } else {
            None
        };
        let mut response = to_historic_task_instance_response(&engine, task);
        response.variables = variables;
        result.push(response);
    }

    Ok(query.paging().paginate(result))
}

/// Max Unicode scalar count for in-memory SQL-LIKE filter operands (tests pin
/// the shared 512 bound from `flowable_engine_common::like`).
#[cfg(test)]
const MAX_SQL_LIKE_LEN: usize = flowable_engine_common::like::MAX_SQL_LIKE_LEN;

/// SQL-LIKE style match for in-memory filters (`%` any sequence, `_` one char,
/// other chars literal). Case-sensitive; callers lower-case both sides for
/// ignore-case variants.
///
/// Space is O(value length) via two rolling rows (not O(n×m) full DP matrix /
/// deep recursion). Thin wrapper over the P143 unified implementation.
fn sql_like_matches(pattern: &str, value: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}

fn sort_historic_task_instances(
    tasks: &mut [HistoricTaskInstance],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    match sort {
        None | Some("id") => tasks.sort_by(|left, right| left.id.cmp(&right.id)),
        Some("name") | Some("taskName") => {
            tasks.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)))
        }
        Some("description") => tasks.sort_by(|left, right| {
            left.description
                .cmp(&right.description)
                .then(left.id.cmp(&right.id))
        }),
        Some("created") | Some("createTime") | Some("startTime") => tasks.sort_by(|left, right| {
            left.start_time
                .cmp(&right.start_time)
                .then(left.id.cmp(&right.id))
        }),
        Some("taskDefinitionKey") => tasks.sort_by(|left, right| {
            left.task_definition_key
                .cmp(&right.task_definition_key)
                .then(left.id.cmp(&right.id))
        }),
        Some("assignee") => tasks.sort_by(|left, right| {
            left.assignee
                .cmp(&right.assignee)
                .then(left.id.cmp(&right.id))
        }),
        Some("owner") => {
            tasks.sort_by(|left, right| left.owner.cmp(&right.owner).then(left.id.cmp(&right.id)))
        }
        Some("category") => tasks.sort_by(|left, right| {
            historic_task_category(left)
                .cmp(&historic_task_category(right))
                .then(left.id.cmp(&right.id))
        }),
        Some("tenantId") => tasks.sort_by(|left, right| {
            historic_task_tenant_id(left)
                .cmp(&historic_task_tenant_id(right))
                .then(left.id.cmp(&right.id))
        }),
        Some("processInstanceId") => tasks.sort_by(|left, right| {
            left.process_instance_id
                .cmp(&right.process_instance_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("executionId") => tasks.sort_by(|left, right| {
            left.execution_id
                .cmp(&right.execution_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("endTime") => tasks.sort_by(|left, right| {
            left.end_time
                .cmp(&right.end_time)
                .then(left.id.cmp(&right.id))
        }),
        Some("priority") => tasks.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then(left.id.cmp(&right.id))
        }),
        Some("dueDate") => tasks.sort_by(|left, right| {
            left.due_date
                .cmp(&right.due_date)
                .then(left.id.cmp(&right.id))
        }),
        Some("duration") | Some("durationInMillis") => tasks.sort_by(|left, right| {
            left.duration_ms
                .cmp(&right.duration_ms)
                .then(left.id.cmp(&right.id))
        }),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported historic task sort field '{other}'"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => tasks.reverse(),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported historic task sort order '{other}'"
            )));
        }
    }

    Ok(())
}

fn historic_task_category(task: &HistoricTaskInstance) -> Option<String> {
    historic_task_string_field(task, "category", "category")
}

fn historic_task_tenant_id(task: &HistoricTaskInstance) -> Option<String> {
    historic_task_string_field(task, "tenantId", "tenant_id")
}

fn historic_task_string_field(
    task: &HistoricTaskInstance,
    camel_case: &str,
    snake_case: &str,
) -> Option<String> {
    let value = serde_json::to_value(task).ok()?;
    value
        .get(camel_case)
        .or_else(|| value.get(snake_case))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricActivityInstanceResponse {
    id: String,
    activity_id: String,
    activity_name: Option<String>,
    activity_type: String,
    process_instance_id: String,
    execution_id: String,
    start_time: String,
    end_time: Option<String>,
    duration_in_millis: Option<i64>,
    assignee: Option<String>,
    /// Java engine `HistoricActivityInstance.getDeleteReason()`; exposed on REST
    /// for observability (event-gateway cancel etc.). Null when completed normally.
    delete_reason: Option<String>,
}

fn to_historic_activity_instance_response(
    activity: HistoricActivityInstance,
) -> HistoricActivityInstanceResponse {
    HistoricActivityInstanceResponse {
        id: activity.id,
        activity_id: activity.activity_id,
        activity_name: activity.activity_name,
        activity_type: activity.activity_type,
        process_instance_id: activity.process_instance_id,
        execution_id: activity.execution_id,
        start_time: activity.start_time.to_rfc3339(),
        end_time: activity.end_time.map(|time| time.to_rfc3339()),
        duration_in_millis: activity.duration_ms,
        assignee: activity.assignee,
        delete_reason: activity.delete_reason,
    }
}

pub(crate) async fn query_historic_activity_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<HistoricActivityInstanceResponse>>, ApiError> {
    let mut query: HistoricActivityInstanceListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: HistoricActivityInstanceListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);

    Ok(Json(historic_activity_instances_for_query(engine, query)?))
}

pub(crate) async fn historic_activity_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricActivityInstanceResponse>>, ApiError> {
    let query: HistoricActivityInstanceListQuery = parse_query(&uri)?;
    Ok(Json(historic_activity_instances_for_query(engine, query)?))
}

fn historic_activity_instances_for_query(
    engine: Arc<ProcessEngine>,
    query: HistoricActivityInstanceListQuery,
) -> Result<PagedResponse<HistoricActivityInstanceResponse>, ApiError> {
    let mut activity_query = engine
        .get_history_service()
        .create_historic_activity_instance_query();
    if let Some(process_instance_id) = query.process_instance_id.clone() {
        activity_query = activity_query.process_instance_id(process_instance_id);
    }
    let mut activities = activity_query.list()?;
    if let Some(activity_instance_id) = query.activity_instance_id.as_deref() {
        activities.retain(|activity| activity.id == activity_instance_id);
    }
    if let Some(execution_id) = query.execution_id.as_deref() {
        activities.retain(|activity| activity.execution_id == execution_id);
    }
    if let Some(activity_id) = query.activity_id.as_deref() {
        activities.retain(|activity| activity.activity_id == activity_id);
    }
    if let Some(activity_name) = query.activity_name.as_deref() {
        activities.retain(|activity| activity.activity_name.as_deref() == Some(activity_name));
    }
    if let Some(activity_name_like) = query.activity_name_like.as_deref() {
        activities.retain(|activity| {
            activity
                .activity_name
                .as_deref()
                .is_some_and(|name| sql_like_matches(activity_name_like, name))
        });
    }
    if let Some(activity_type) = query.activity_type.as_deref() {
        activities.retain(|activity| activity.activity_type.eq_ignore_ascii_case(activity_type));
    }
    if let Some(finished) = query.finished {
        activities.retain(|activity| activity.end_time.is_some() == finished);
    }
    if query.unfinished == Some(true) {
        activities.retain(|activity| activity.end_time.is_none());
    }
    if let Some(started_after) = query.started_after.as_deref() {
        let started_after = parse_timestamp_millis(started_after)?;
        activities.retain(|activity| activity.start_time.timestamp_millis() >= started_after);
    }
    if let Some(started_before) = query.started_before.as_deref() {
        let started_before = parse_timestamp_millis(started_before)?;
        activities.retain(|activity| activity.start_time.timestamp_millis() <= started_before);
    }
    if let Some(finished_after) = query.finished_after.as_deref() {
        let finished_after = parse_timestamp_millis(finished_after)?;
        activities.retain(|activity| {
            activity
                .end_time
                .is_some_and(|end_time| end_time.timestamp_millis() >= finished_after)
        });
    }
    if let Some(finished_before) = query.finished_before.as_deref() {
        let finished_before = parse_timestamp_millis(finished_before)?;
        activities.retain(|activity| {
            activity
                .end_time
                .is_some_and(|end_time| end_time.timestamp_millis() <= finished_before)
        });
    }
    if let Some(ids) = query
        .process_instance_ids
        .as_ref()
        .filter(|ids| !ids.is_empty())
    {
        activities.retain(|activity| ids.contains(&activity.process_instance_id));
    }
    if let Some(task_assignee) = query.task_assignee.as_deref() {
        activities.retain(|activity| activity.assignee.as_deref() == Some(task_assignee));
    }
    if query.process_definition_id.is_some()
        || query.tenant_id.is_some()
        || query.tenant_id_like.is_some()
        || query.without_tenant_id == Some(true)
    {
        let definition_by_instance: HashMap<String, String> = engine
            .get_history_service()
            .create_historic_process_instance_query()
            .list()?
            .into_iter()
            .map(|instance| (instance.id, instance.process_definition_id))
            .collect();
        if let Some(process_definition_id) = query.process_definition_id.as_deref() {
            activities.retain(|activity| {
                definition_by_instance
                    .get(&activity.process_instance_id)
                    .map(String::as_str)
                    == Some(process_definition_id)
            });
        }
        let definitions = historic_process_definition_meta(&engine)?;
        let tenant_of = |activity: &HistoricActivityInstance| -> Option<String> {
            definition_by_instance
                .get(&activity.process_instance_id)
                .and_then(|definition_id| definitions.get(definition_id))
                .and_then(|definition| definition.tenant_id.clone())
        };
        if let Some(tenant_id) = query.tenant_id.as_deref() {
            activities.retain(|activity| tenant_of(activity).as_deref() == Some(tenant_id));
        }
        if let Some(pattern) = query.tenant_id_like.as_deref() {
            activities.retain(|activity| {
                tenant_of(activity).is_some_and(|tenant_id| sql_like_matches(pattern, &tenant_id))
            });
        }
        if query.without_tenant_id == Some(true) {
            activities.retain(|activity| tenant_of(activity).is_none());
        }
    }
    if query
        .called_process_instance_ids
        .as_ref()
        .is_some_and(|ids| !ids.is_empty())
    {
        // Data limitation: the engine records no CALLED_PROCESS_INSTANCE_ID_
        // on historic activities, so this filter can never match.
        activities.clear();
    }
    sort_historic_activity_instances(
        &mut activities,
        query.sort.as_deref(),
        query.order.as_deref(),
    )?;

    let result = activities
        .into_iter()
        .map(to_historic_activity_instance_response)
        .collect();

    Ok(query.paging().paginate(result))
}

fn sort_historic_activity_instances(
    activities: &mut [HistoricActivityInstance],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    match sort {
        None | Some("activityId") => activities.sort_by(|left, right| {
            left.activity_id
                .cmp(&right.activity_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("activityInstanceId") | Some("id") => {
            activities.sort_by(|left, right| left.id.cmp(&right.id))
        }
        Some("activityName") => activities.sort_by(|left, right| {
            left.activity_name
                .cmp(&right.activity_name)
                .then(left.id.cmp(&right.id))
        }),
        Some("activityType") => activities.sort_by(|left, right| {
            left.activity_type
                .cmp(&right.activity_type)
                .then(left.id.cmp(&right.id))
        }),
        Some("processInstanceId") => activities.sort_by(|left, right| {
            left.process_instance_id
                .cmp(&right.process_instance_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("executionId") => activities.sort_by(|left, right| {
            left.execution_id
                .cmp(&right.execution_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("startTime") => activities.sort_by(|left, right| {
            left.start_time
                .cmp(&right.start_time)
                .then(left.id.cmp(&right.id))
        }),
        Some("endTime") => activities.sort_by(|left, right| {
            left.end_time
                .cmp(&right.end_time)
                .then(left.id.cmp(&right.id))
        }),
        Some("duration") | Some("durationInMillis") => activities.sort_by(|left, right| {
            left.duration_ms
                .cmp(&right.duration_ms)
                .then(left.id.cmp(&right.id))
        }),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported historic activity sort field '{other}'"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => activities.reverse(),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported historic activity sort order '{other}'"
            )));
        }
    }

    Ok(())
}

fn historic_process_variable_instances(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Result<Vec<HistoricVariableInstance>, ApiError> {
    Ok(engine
        .get_history_service()
        .create_historic_variable_instance_query()
        .process_instance_id(process_instance_id.to_string())
        .exclude_task_variables()
        .list()?)
}

fn historic_task_variable_responses(
    engine: &ProcessEngine,
    task: &HistoricTaskInstance,
    include_process_variables: bool,
    include_task_local_variables: bool,
) -> Result<Vec<super::process_instances::RestVariableResponse>, ApiError> {
    let mut responses = Vec::new();
    if include_task_local_variables {
        responses.extend(historic_variable_response_scope(
            engine
                .get_history_service()
                .create_historic_variable_instance_query()
                .task_id(task.id.clone())
                .list()?,
            "local",
        ));
    }
    if include_process_variables {
        responses.extend(historic_variable_response_scope(
            historic_process_variable_instances(engine, &task.process_instance_id)?,
            "global",
        ));
    }
    responses.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.scope.cmp(&right.scope))
    });
    Ok(responses)
}

#[derive(Debug, Clone, Copy)]
enum HistoricTaskVariableScope {
    TaskLocal,
    ProcessInstance,
}

fn apply_historic_task_variable_filters(
    tasks: &mut Vec<HistoricTaskInstance>,
    variables: &[HistoricVariableInstance],
    filters: &[HistoricQueryVariable],
    scope: HistoricTaskVariableScope,
) -> Result<(), ApiError> {
    for filter in filters {
        // Java HistoricTaskInstanceBaseResource rejects value-only variable
        // filters outright (unlike the historic process instance query).
        if filter.name.is_none() {
            return Err(ApiError::bad_request(
                "Value-only query (without a variable-name) is not supported.",
            ));
        }
        let operation = parse_historic_query_variable_operation(filter)?;
        let value = historic_query_variable_value(filter)?;
        let name = filter.name.as_deref();

        validate_historic_query_variable(filter, operation, value)?;

        tasks.retain(|task| {
            variables
                .iter()
                .filter(|variable| historic_variable_in_task_scope(variable, task, scope))
                .any(|variable| {
                    historic_variable_matches(
                        variable.name.as_str(),
                        &variable.value,
                        name,
                        operation,
                        value,
                    )
                })
        });
    }

    Ok(())
}

fn apply_historic_process_instance_variable_filters(
    instances: &mut Vec<HistoricProcessInstance>,
    variables: &[HistoricVariableInstance],
    filters: &[HistoricQueryVariable],
) -> Result<(), ApiError> {
    for filter in filters {
        let operation = parse_historic_query_variable_operation(filter)?;
        let value = historic_query_variable_value(filter)?;
        let name = filter.name.as_deref();

        validate_historic_query_variable(filter, operation, value)?;

        instances.retain(|instance| {
            variables
                .iter()
                .filter(|variable| {
                    variable.process_instance_id == instance.id && variable.task_id.is_none()
                })
                .any(|variable| {
                    historic_variable_matches(
                        variable.name.as_str(),
                        &variable.value,
                        name,
                        operation,
                        value,
                    )
                })
        });
    }

    Ok(())
}

fn historic_variable_in_task_scope(
    variable: &HistoricVariableInstance,
    task: &HistoricTaskInstance,
    scope: HistoricTaskVariableScope,
) -> bool {
    match scope {
        HistoricTaskVariableScope::TaskLocal => {
            variable.task_id.as_deref() == Some(task.id.as_str())
        }
        HistoricTaskVariableScope::ProcessInstance => {
            variable.process_instance_id == task.process_instance_id && variable.task_id.is_none()
        }
    }
}

fn parse_historic_query_variable_operation(
    variable: &HistoricQueryVariable,
) -> Result<QueryVariableOperation, ApiError> {
    match variable.operation.as_deref() {
        None => Err(ApiError::bad_request(format!(
            "Variable operation is missing for variable: {}",
            variable.name.as_deref().unwrap_or("null")
        ))),
        Some(name) => QueryVariableOperation::from_friendly_name(name).ok_or_else(|| {
            ApiError::bad_request(format!("Unsupported variable query operation: {name}"))
        }),
    }
}

fn historic_query_variable_value(variable: &HistoricQueryVariable) -> Result<&Value, ApiError> {
    variable.value.as_ref().ok_or_else(|| {
        ApiError::bad_request(format!(
            "Variable value is missing for variable: {}",
            variable.name.as_deref().unwrap_or("null")
        ))
    })
}

fn validate_historic_query_variable(
    variable: &HistoricQueryVariable,
    operation: QueryVariableOperation,
    value: &Value,
) -> Result<(), ApiError> {
    validate_name_less_equals(variable.name.as_deref(), operation)?;
    validate_operation_value(operation, value)
}

fn historic_variable_matches(
    candidate_name: &str,
    candidate_value: &Value,
    expected_name: Option<&str>,
    operation: QueryVariableOperation,
    expected_value: &Value,
) -> bool {
    if expected_name.is_some_and(|name| candidate_name != name) {
        return false;
    }

    value_matches(candidate_value, operation, expected_value)
}

fn historic_variable_response_scope(
    variables: Vec<HistoricVariableInstance>,
    scope: &str,
) -> Vec<super::process_instances::RestVariableResponse> {
    let mut responses = variables
        .into_iter()
        .map(|variable| {
            let mut response =
                super::process_instances::to_rest_variable_response(variable.name, variable.value);
            response.scope = scope.to_string();
            response
        })
        .collect::<Vec<_>>();
    responses.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.scope.cmp(&right.scope))
    });
    responses
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricVariableInstanceResponse {
    id: String,
    process_instance_id: String,
    execution_id: Option<String>,
    task_id: Option<String>,
    name: String,
    variable_type: String,
    value: serde_json::Value,
    create_time: String,
    last_updated_time: String,
}

fn to_historic_variable_instance_response(
    variable: HistoricVariableInstance,
) -> HistoricVariableInstanceResponse {
    let variable_type =
        persisted_or_inferred_variable_type(&variable.variable_type, &variable.value);
    HistoricVariableInstanceResponse {
        id: variable.id,
        process_instance_id: variable.process_instance_id,
        execution_id: variable.execution_id,
        task_id: variable.task_id,
        name: variable.name,
        variable_type,
        value: variable.value,
        create_time: variable.create_time.to_rfc3339(),
        last_updated_time: variable.last_updated_time.to_rfc3339(),
    }
}

pub(crate) async fn query_historic_variable_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<HistoricVariableInstanceResponse>>, ApiError> {
    let mut query: HistoricVariableListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: HistoricVariableListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);

    Ok(Json(historic_variable_instances_for_query(engine, query)?))
}

pub(crate) async fn historic_variable_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricVariableInstanceResponse>>, ApiError> {
    let query: HistoricVariableListQuery = parse_query(&uri)?;
    Ok(Json(historic_variable_instances_for_query(engine, query)?))
}

fn historic_variable_instances_for_query(
    engine: Arc<ProcessEngine>,
    query: HistoricVariableListQuery,
) -> Result<PagedResponse<HistoricVariableInstanceResponse>, ApiError> {
    let mut variable_query = engine
        .get_history_service()
        .create_historic_variable_instance_query();
    if let Some(process_instance_id) = query.process_instance_id.clone() {
        variable_query = variable_query.process_instance_id(process_instance_id);
    }
    if let Some(execution_id) = query.execution_id.clone() {
        variable_query = variable_query.execution_id(execution_id);
    }
    if let Some(task_id) = query.task_id.clone() {
        variable_query = variable_query.task_id(task_id);
    }
    if let Some(variable_name) = query.variable_name.clone() {
        variable_query = variable_query.variable_name(variable_name);
    }
    if let Some(variable_name_like) = query.variable_name_like.clone() {
        variable_query = variable_query.variable_name_like(variable_name_like);
    }
    if query.exclude_task_variables == Some(true) {
        variable_query = variable_query.exclude_task_variables();
    }
    let mut variables = variable_query.list()?;
    if let Some(variable_type) = query.variable_type.as_deref() {
        variables.retain(|variable| {
            persisted_or_inferred_variable_type(&variable.variable_type, &variable.value)
                == variable_type
        });
    }
    sort_historic_variable_instances(
        &mut variables,
        query.sort.as_deref(),
        query.order.as_deref(),
    )?;

    let result = variables
        .into_iter()
        .map(to_historic_variable_instance_response)
        .collect();

    Ok(query.paging().paginate(result))
}

fn sort_historic_variable_instances(
    variables: &mut [HistoricVariableInstance],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    let descending = match order {
        None | Some("asc") => false,
        Some("desc") => true,
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported historic variable instance sort order '{other}'"
            )));
        }
    };

    match sort {
        None | Some("variableName") | Some("name") => variables.sort_by(|left, right| {
            historic_variable_sort_order(left.name.cmp(&right.name), left, right, descending)
        }),
        Some("variableType") | Some("type") => variables.sort_by(|left, right| {
            historic_variable_sort_order(
                persisted_or_inferred_variable_type(&left.variable_type, &left.value).cmp(
                    &persisted_or_inferred_variable_type(&right.variable_type, &right.value),
                ),
                left,
                right,
                descending,
            )
        }),
        Some("processInstanceId") => variables.sort_by(|left, right| {
            historic_variable_sort_order(
                left.process_instance_id.cmp(&right.process_instance_id),
                left,
                right,
                descending,
            )
        }),
        Some("executionId") => variables.sort_by(|left, right| {
            historic_variable_sort_order(
                left.execution_id.cmp(&right.execution_id),
                left,
                right,
                descending,
            )
        }),
        Some("taskId") => variables.sort_by(|left, right| {
            historic_variable_sort_order(left.task_id.cmp(&right.task_id), left, right, descending)
        }),
        Some("createTime") => variables.sort_by(|left, right| {
            historic_variable_sort_order(
                left.create_time.cmp(&right.create_time),
                left,
                right,
                descending,
            )
        }),
        Some("lastUpdatedTime") => variables.sort_by(|left, right| {
            historic_variable_sort_order(
                left.last_updated_time.cmp(&right.last_updated_time),
                left,
                right,
                descending,
            )
        }),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported historic variable instance sort field '{other}'"
            )));
        }
    }

    Ok(())
}

fn persisted_or_inferred_variable_type(persisted_type: &str, value: &Value) -> String {
    if matches!(persisted_type, "binary" | "bytes" | "serializable") {
        persisted_type.to_string()
    } else {
        rest_variable_type(value).to_string()
    }
}

fn historic_variable_sort_order(
    primary: std::cmp::Ordering,
    left: &HistoricVariableInstance,
    right: &HistoricVariableInstance,
    descending: bool,
) -> std::cmp::Ordering {
    if descending {
        primary.reverse().then(left.id.cmp(&right.id))
    } else {
        primary.then(left.id.cmp(&right.id))
    }
}

pub(crate) async fn get_historic_variable_instance_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(variable_instance_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let variable = engine
        .get_runtime_store()
        .get_historic_variable_instance(&variable_instance_id, &mut session)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic variable instance '{}' was not found",
                variable_instance_id
            ))
        })?;

    Ok(Json(variable.value))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryCleanupCommand {
    pub before_date: Option<String>,
    pub process_instance_ids: Option<Vec<String>>,
    pub cleanup_type: Option<String>,
    pub batch_size: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CleanupResultResponse {
    pub deleted_process_instances: usize,
    pub deleted_task_instances: usize,
    pub deleted_activity_instances: usize,
    pub deleted_variable_instances: usize,
    pub deleted_details: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CleanupStrategy {
    pub retention_days: Option<u32>,
    pub max_records: Option<usize>,
    pub auto_cleanup: Option<bool>,
    pub cleanup_schedule: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CleanupStrategyResponse {
    pub retention_days: Option<u32>,
    pub max_records: Option<usize>,
    pub auto_cleanup: bool,
    pub cleanup_schedule: Option<String>,
    pub configured_at: String,
}

pub(crate) async fn cleanup_history(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(command): Json<HistoryCleanupCommand>,
) -> Result<Json<CleanupResultResponse>, ApiError> {
    let start_time = std::time::Instant::now();

    let before_date = match command.before_date.as_deref() {
        Some(date_str) => Some(parse_timestamp_millis(date_str)?),
        None => None,
    };

    let cleanup_type = command.cleanup_type.as_deref().unwrap_or("all");
    if !matches!(cleanup_type, "all" | "completed" | "terminated") {
        return Err(ApiError::BadRequest(format!(
            "Invalid cleanup type '{}'. Must be one of: all, completed, terminated",
            cleanup_type
        )));
    }

    let batch_size = command.batch_size.unwrap_or(100);
    if batch_size == 0 || batch_size > 10000 {
        return Err(ApiError::BadRequest(
            "Batch size must be between 1 and 10000".to_string(),
        ));
    }

    let mut deleted_process_instances = 0;
    let deleted_task_instances = 0;
    let deleted_activity_instances = 0;
    let deleted_variable_instances = 0;
    let deleted_details = 0;

    let mut session = engine.get_runtime_store().create_session().unwrap();
    if let Some(process_instance_ids) = command.process_instance_ids {
        for chunk in process_instance_ids.chunks(batch_size) {
            for id in chunk {
                if let Some(instance) = engine
                    .get_runtime_store()
                    .get_historic_process_instance(id, &mut session)
                {
                    // P133: only finished instances; cutoff is end_time
                    // (Java DefaultHistoryCleaningManager.java:36 finishedBefore)
                    let should_delete = match cleanup_type {
                        "completed" => instance.end_time.is_some(),
                        "terminated" => {
                            instance.end_time.is_some()
                                && instance
                                    .delete_reason
                                    .as_deref()
                                    .is_some_and(|r| r.contains("terminated"))
                        }
                        // "all": still requires end_time (running instances never deleted)
                        _ => instance.end_time.is_some(),
                    };

                    if should_delete {
                        // P133: cutoff on end_time, not start_time
                        if before_date.is_some_and(|before| {
                            instance
                                .end_time
                                .is_none_or(|end| end.timestamp_millis() >= before)
                        }) {
                            continue;
                        }
                        engine
                            .get_runtime_store()
                            .delete_historic_process_instance_cascade(id, &mut session);
                        deleted_process_instances += 1;
                    }
                }
            }
        }
    } else if let Some(before) = before_date {
        let all_instances = engine
            .get_history_service()
            .create_historic_process_instance_query()
            .list()?;

        let instances_to_delete: Vec<String> = all_instances
            .iter()
            .filter(|instance| {
                // P133: end_time cutoff (Java DefaultHistoryCleaningManager.java:36)
                let date_match = instance
                    .end_time
                    .is_some_and(|end| end.timestamp_millis() < before);
                let type_match = match cleanup_type {
                    "completed" => instance.end_time.is_some(),
                    "terminated" => {
                        instance.end_time.is_some()
                            && instance
                                .delete_reason
                                .as_deref()
                                .is_some_and(|r| r.contains("terminated"))
                    }
                    // "all": date_match already requires end_time
                    _ => true,
                };
                date_match && type_match
            })
            .take(batch_size)
            .map(|instance| instance.id.clone())
            .collect();

        for id in &instances_to_delete {
            engine
                .get_runtime_store()
                .delete_historic_process_instance_cascade(id, &mut session);
            deleted_process_instances += 1;
        }
    }
    session.flush_and_commit().unwrap();

    let duration = start_time.elapsed();
    let duration_ms = duration.as_millis() as u64;

    let log = flowable_engine::history::historic_entities::CleanupLog {
        id: uuid::Uuid::new_v4().to_string(),
        cleanup_type: cleanup_type.to_string(),
        before_date: before_date
            .map(|ts| chrono::DateTime::from_timestamp_millis(ts).unwrap_or_default()),
        records_deleted: deleted_process_instances,
        duration_ms,
        status: "success".to_string(),
        error_message: None,
        timestamp: chrono::Utc::now(),
    };
    {
        let mut session = engine.get_runtime_store().create_session().unwrap();
        engine
            .get_runtime_store()
            .insert_cleanup_log(log, &mut session);
        session.flush_and_commit().unwrap();
    }

    Ok(Json(CleanupResultResponse {
        deleted_process_instances,
        deleted_task_instances,
        deleted_activity_instances,
        deleted_variable_instances,
        deleted_details,
        duration_ms,
    }))
}

pub(crate) async fn configure_cleanup_strategy(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(strategy): Json<CleanupStrategy>,
) -> Result<Json<CleanupStrategyResponse>, ApiError> {
    if let Some(retention_days) = strategy.retention_days
        && (retention_days == 0 || retention_days > 3650)
    {
        return Err(ApiError::BadRequest(
            "Retention days must be between 1 and 3650".to_string(),
        ));
    }

    if let Some(max_records) = strategy.max_records
        && (max_records == 0 || max_records > 10_000_000)
    {
        return Err(ApiError::BadRequest(
            "Max records must be between 1 and 10000000".to_string(),
        ));
    }

    if let Some(schedule) = strategy.cleanup_schedule.as_deref()
        && !schedule.is_empty()
        && !is_valid_cron_like(schedule)
    {
        return Err(ApiError::BadRequest(
            "Invalid cron expression format".to_string(),
        ));
    }

    let config = flowable_engine::history::historic_entities::CleanupStrategyConfig {
        retention_days: strategy.retention_days,
        max_records: strategy.max_records,
        auto_cleanup: strategy.auto_cleanup.unwrap_or(false),
        cleanup_schedule: strategy.cleanup_schedule.clone(),
    };
    {
        let mut session = engine.get_runtime_store().create_session().unwrap();
        engine
            .get_runtime_store()
            .set_cleanup_strategy_config(&config, &mut session);
        session.flush_and_commit().unwrap();
    }

    Ok(Json(CleanupStrategyResponse {
        retention_days: strategy.retention_days,
        max_records: strategy.max_records,
        auto_cleanup: strategy.auto_cleanup.unwrap_or(false),
        cleanup_schedule: strategy.cleanup_schedule,
        configured_at: chrono::Utc::now().to_rfc3339(),
    }))
}

fn is_valid_cron_like(expression: &str) -> bool {
    let parts: Vec<&str> = expression.split_whitespace().collect();
    parts.len() == 5
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn query_variable(body: Value) -> HistoricQueryVariable {
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn historic_variable_parse_accepts_all_ten_operations() {
        for name in [
            "equals",
            "notEquals",
            "equalsIgnoreCase",
            "notEqualsIgnoreCase",
            "like",
            "likeIgnoreCase",
            "greaterThan",
            "greaterThanOrEquals",
            "lessThan",
            "lessThanOrEquals",
        ] {
            let variable = query_variable(json!({"name": "v", "operation": name, "value": "x"}));
            let parsed = parse_historic_query_variable_operation(&variable).unwrap();
            assert_eq!(
                QueryVariableOperation::from_friendly_name(name),
                Some(parsed),
                "operation {name}"
            );
        }
    }

    #[test]
    fn historic_variable_illegal_operation_is_400() {
        let variable = query_variable(json!({"name": "v", "operation": "bogusOp", "value": 1}));
        let error = parse_historic_query_variable_operation(&variable).unwrap_err();
        assert!(matches!(
            error,
            ApiError::BadRequest(message) if message == "Unsupported variable query operation: bogusOp"
        ));
    }

    #[test]
    fn historic_variable_nameless_non_equals_and_boolean_comparison_are_400() {
        let nameless = query_variable(json!({"operation": "notEquals", "value": "x"}));
        let error = validate_historic_query_variable(
            &nameless,
            QueryVariableOperation::NotEquals,
            &json!("x"),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ApiError::BadRequest(message) if message ==
                "Value-only query (without a variable-name) is only supported when using 'equals' operation."
        ));

        let bool_comp = query_variable(json!({"name": "v", "value": true}));
        let error = validate_historic_query_variable(
            &bool_comp,
            QueryVariableOperation::GreaterThan,
            &json!(true),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ApiError::BadRequest(message) if message ==
                "Booleans and null cannot be used in 'greater than' condition"
        ));
    }

    #[test]
    fn historic_variable_like_greater_ignore_case_positive_and_miss() {
        // like: % wildcard matches, miss otherwise.
        assert!(historic_variable_matches(
            "v",
            &json!("HelloWorld"),
            Some("v"),
            QueryVariableOperation::Like,
            &json!("Hello%")
        ));
        assert!(!historic_variable_matches(
            "v",
            &json!("HelloWorld"),
            Some("v"),
            QueryVariableOperation::Like,
            &json!("Nope%")
        ));
        // greaterThan: numeric, miss on equal.
        assert!(historic_variable_matches(
            "n",
            &json!(10),
            Some("n"),
            QueryVariableOperation::GreaterThan,
            &json!(5)
        ));
        assert!(!historic_variable_matches(
            "n",
            &json!(10),
            Some("n"),
            QueryVariableOperation::GreaterThan,
            &json!(10)
        ));
        // equalsIgnoreCase: miss on differing value.
        assert!(historic_variable_matches(
            "s",
            &json!("Hello"),
            Some("s"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("hello")
        ));
        assert!(!historic_variable_matches(
            "s",
            &json!("Hello"),
            Some("s"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("world")
        ));
        // Expected-name mismatch never matches.
        assert!(!historic_variable_matches(
            "s",
            &json!("Hello"),
            Some("other"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("hello")
        ));
    }

    /// Semantic pin tests for in-memory SQL-LIKE (`%` / `_` / literal).
    /// Argument order in history.rs is `(pattern, value)`.
    #[test]
    fn sql_like_semantic_pins() {
        // empty
        assert!(sql_like_matches("", ""));
        assert!(!sql_like_matches("", "a"));
        assert!(!sql_like_matches("a", ""));
        assert!(sql_like_matches("%", ""));
        assert!(sql_like_matches("%%", ""));
        assert!(!sql_like_matches("_", ""));

        // literal
        assert!(sql_like_matches("abc", "abc"));
        assert!(!sql_like_matches("ab", "abc"));
        assert!(!sql_like_matches("abc", "ab"));
        assert!(!sql_like_matches("Abc", "abc")); // case-sensitive
        assert!(!sql_like_matches("abd", "abc"));

        // `%` any sequence
        assert!(sql_like_matches("%", "hello"));
        assert!(sql_like_matches("h%", "hello"));
        assert!(sql_like_matches("%o", "hello"));
        assert!(sql_like_matches("%ell%", "hello"));
        assert!(sql_like_matches("h%o", "hello"));
        assert!(sql_like_matches("%%", "hello"));
        assert!(sql_like_matches("%h%e%l%o%", "hello"));
        assert!(!sql_like_matches("x%", "hello"));
        assert!(!sql_like_matches("%x", "hello"));

        // `_` single character
        assert!(sql_like_matches("_", "a"));
        assert!(sql_like_matches("a_", "ab"));
        assert!(sql_like_matches("_b", "ab"));
        assert!(sql_like_matches("a_c", "abc"));
        assert!(!sql_like_matches("_", "ab"));
        assert!(!sql_like_matches("__", "a"));
        assert!(!sql_like_matches("_", ""));

        // mixed + pattern longer than value
        assert!(sql_like_matches("%_%", "ab"));
        assert!(sql_like_matches("%%_%%", "x"));
        assert!(!sql_like_matches("a_c", "ab"));
        assert!(!sql_like_matches("a_", "a"));
        assert!(!sql_like_matches("abc%", "ab"));

        // Unicode is one char for `_`
        assert!(sql_like_matches("_", "你"));
        assert!(sql_like_matches("你_", "你好"));
        assert!(!sql_like_matches("_", "你好"));
    }

    #[test]
    fn sql_like_rejects_oversized_without_huge_allocation() {
        let long_value = "v".repeat(MAX_SQL_LIKE_LEN + 1);
        let long_pattern = "%".repeat(MAX_SQL_LIKE_LEN + 1);
        assert!(!sql_like_matches(&long_pattern, &long_value));
        assert!(!sql_like_matches("%", &long_value));
        assert!(!sql_like_matches(&long_pattern, "ok"));
        let at_cap_v = "a".repeat(MAX_SQL_LIKE_LEN);
        let at_cap_p = "%".repeat(MAX_SQL_LIKE_LEN);
        assert!(sql_like_matches(&at_cap_p, &at_cap_v));
        assert!(sql_like_matches("%", &at_cap_v));
    }
}
