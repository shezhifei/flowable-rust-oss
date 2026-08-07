use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    body::{Body, Bytes},
    extract::{Path, Query, Request},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use flowable_engine::cmd::execution_variable_cmd::{
    ExecutionVariableMutation, ExecutionVariableScope,
};
use flowable_engine::cmd::task_variable_cmd::VariableMutationMode;
use flowable_engine::cmd::trigger_start_event_subscription_cmd::TriggerProcessStartByEventCmd;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::runtime_service::ActivityMigrationMapping as RuntimeActivityMigrationMapping;
use flowable_engine::engine::task_service::TaskUpdate;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_engine::persistence::runtime_store::{EventSubscriptionKind, RuntimeEventWaitKind};
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::{ProcessInstance, ProcessInstanceUpdate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::variable_types::{convert_explicit_variable_value, rest_variable_type};

const PROCESS_INSTANCES_PATH: &str = "/runtime/process-instances";
const PROCESS_INSTANCES_DELETE_PATH: &str = "/runtime/process-instances/delete";
const PROCESS_INSTANCE_PATH: &str = "/runtime/process-instances/:process_instance_id";
const PROCESS_INSTANCE_INJECT_PATH: &str = "/runtime/process-instances/:process_instance_id/inject";
const PROCESS_INSTANCE_VALIDATE_MIGRATION_PATH: &str =
    "/runtime/process-instances/:process_instance_id/validate-migration";
const PROCESS_INSTANCE_MIGRATE_PATH: &str =
    "/runtime/process-instances/:process_instance_id/migrate";
const PROCESS_INSTANCE_EVALUATE_CONDITIONS_PATH: &str =
    "/runtime/process-instances/:process_instance_id/evaluate-conditions";
const PROCESS_INSTANCE_CHANGE_STATE_PATH: &str =
    "/runtime/process-instances/:process_instance_id/change-state";
const PROCESS_INSTANCE_VARIABLES_PATH: &str =
    "/runtime/process-instances/:process_instance_id/variables";
const PROCESS_INSTANCE_VARIABLE_PATH: &str =
    "/runtime/process-instances/:process_instance_id/variables/:variable_name";
const PROCESS_INSTANCE_VARIABLES_ASYNC_PATH: &str =
    "/runtime/process-instances/:process_instance_id/variables-async";
const PROCESS_INSTANCE_VARIABLE_ASYNC_PATH: &str =
    "/runtime/process-instances/:process_instance_id/variables-async/:variable_name";
const PROCESS_INSTANCE_VARIABLE_DATA_PATH: &str =
    "/runtime/process-instances/:process_instance_id/variables/:variable_name/data";
const PROCESS_INSTANCE_MODIFICATION_PATH: &str =
    "/runtime/process-instances/:process_instance_id/modification";
// Rust extension: Java TaskService has processInstanceId attachment APIs but
// the BPMN REST module has no process-instance attachment collection resource.
const PROCESS_INSTANCE_ATTACHMENTS_PATH: &str =
    "/runtime/process-instances/:process_instance_id/attachments";
const PROCESS_INSTANCE_ATTACHMENT_PATH: &str =
    "/runtime/process-instances/:process_instance_id/attachments/:attachment_id";
const PROCESS_INSTANCE_ATTACHMENT_CONTENT_PATH: &str =
    "/runtime/process-instances/:process_instance_id/attachments/:attachment_id/content";
const EXECUTIONS_PATH: &str = "/runtime/executions";
const EXECUTION_PATH: &str = "/runtime/executions/:execution_id";
const EXECUTION_CHANGE_STATE_PATH: &str = "/runtime/executions/:execution_id/change-state";
const EXECUTION_ACTIVATE_ACTIVITY_PATH: &str =
    "/runtime/executions/:execution_id/activate-activity";
const EXECUTION_ACTIVITIES_PATH: &str = "/runtime/executions/:execution_id/activities";
const EXECUTION_VARIABLES_PATH: &str = "/runtime/executions/:execution_id/variables";
const EXECUTION_VARIABLE_PATH: &str = "/runtime/executions/:execution_id/variables/:variable_name";
const EXECUTION_VARIABLES_ASYNC_PATH: &str = "/runtime/executions/:execution_id/variables-async";
const EXECUTION_VARIABLE_ASYNC_PATH: &str =
    "/runtime/executions/:execution_id/variables-async/:variable_name";
const EXECUTION_VARIABLE_DATA_PATH: &str =
    "/runtime/executions/:execution_id/variables/:variable_name/data";
const EXECUTION_SIGNAL_EVENT_RECEIVED_PATH: &str =
    "/runtime/executions/:execution_id/signal-event-received";
const EXECUTION_MESSAGE_EVENT_RECEIVED_PATH: &str =
    "/runtime/executions/:execution_id/message-event-received";
const ACTIVITY_INSTANCES_PATH: &str = "/runtime/activity-instances";
const PROCESS_INSTANCES_QUERY_PATH: &str = "/query/process-instances";
const EXECUTIONS_QUERY_PATH: &str = "/query/executions";
const ACTIVITY_INSTANCES_QUERY_PATH: &str = "/query/activity-instances";
const VARIABLE_INSTANCES_QUERY_PATH: &str = "/query/variable-instances";
const VARIABLE_INSTANCES_PATH: &str = "/runtime/variable-instances";
const VARIABLE_INSTANCE_DATA_PATH: &str = "/runtime/variable-instances/:variable_instance_id/data";

pub fn router(content_service: super::content::DynContentService) -> Router {
    router_with_prefix("", content_service)
}

fn router_with_prefix(prefix: &str, content_service: super::content::DynContentService) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{PROCESS_INSTANCES_PATH}"),
            post(start).get(super::process_instances_query::list_process_instances),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCES_DELETE_PATH}"),
            post(bulk_delete_process_instances),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_PATH}"),
            get(get_process_instance)
                .put(update_process_instance)
                .delete(delete_process_instance),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_INJECT_PATH}"),
            post(inject_process_instance_activity),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_VALIDATE_MIGRATION_PATH}"),
            post(validate_process_instance_migration),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_MIGRATE_PATH}"),
            post(migrate_process_instance),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_EVALUATE_CONDITIONS_PATH}"),
            post(evaluate_process_instance_conditions),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_CHANGE_STATE_PATH}"),
            post(change_process_instance_state),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_VARIABLES_PATH}"),
            get(list_process_instance_variables)
                .post(create_process_instance_variables)
                .put(update_process_instance_variables)
                .delete(delete_process_instance_variables),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_VARIABLE_PATH}"),
            get(get_process_instance_variable)
                .put(update_process_instance_variable)
                .delete(delete_process_instance_variable),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_VARIABLES_ASYNC_PATH}"),
            post(create_process_instance_variables_async)
                .put(update_process_instance_variables_async),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_VARIABLE_ASYNC_PATH}"),
            put(update_process_instance_variable_async),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_VARIABLE_DATA_PATH}"),
            get(get_process_instance_variable_data).put(update_process_instance_variable_data),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_MODIFICATION_PATH}"),
            post(modify_process_instance),
        )
        // Owned Rust extension (see PROCESS_INSTANCE_ATTACHMENTS_PATH docs).
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_ATTACHMENTS_PATH}"),
            get(list_process_attachments).post(create_process_attachment),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_ATTACHMENT_PATH}"),
            get(get_process_attachment).delete(delete_process_attachment),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_ATTACHMENT_CONTENT_PATH}"),
            get(get_process_attachment_content),
        )
        .route(
            &format!("{prefix}{EXECUTIONS_PATH}"),
            get(super::process_instances_query::list_executions)
                .put(execute_execution_collection_action),
        )
        .route(
            &format!("{prefix}{EXECUTION_PATH}"),
            get(super::process_instances_query::get_execution).put(perform_execution_action),
        )
        .route(
            &format!("{prefix}{EXECUTION_CHANGE_STATE_PATH}"),
            post(change_execution_state),
        )
        .route(
            &format!("{prefix}{EXECUTION_ACTIVATE_ACTIVITY_PATH}"),
            post(execution_activate_activity),
        )
        .route(
            &format!("{prefix}{EXECUTION_ACTIVITIES_PATH}"),
            get(super::process_instances_query::get_execution_active_activities),
        )
        .route(
            &format!("{prefix}{EXECUTION_VARIABLES_PATH}"),
            get(list_execution_variables)
                .post(create_execution_variables)
                .put(update_execution_variables)
                .delete(delete_all_local_execution_variables),
        )
        .route(
            &format!("{prefix}{EXECUTION_VARIABLE_PATH}"),
            get(get_execution_variable)
                .put(update_execution_variable)
                .delete(delete_execution_variable),
        )
        .route(
            &format!("{prefix}{EXECUTION_VARIABLES_ASYNC_PATH}"),
            post(create_execution_variables_async).put(update_execution_variables_async),
        )
        .route(
            &format!("{prefix}{EXECUTION_VARIABLE_ASYNC_PATH}"),
            put(update_execution_variable_async),
        )
        .route(
            &format!("{prefix}{EXECUTION_VARIABLE_DATA_PATH}"),
            get(get_execution_variable_data).put(update_execution_variable_data),
        )
        .route(
            &format!("{prefix}{EXECUTION_SIGNAL_EVENT_RECEIVED_PATH}"),
            post(execution_signal_event_received),
        )
        .route(
            &format!("{prefix}{EXECUTION_MESSAGE_EVENT_RECEIVED_PATH}"),
            post(execution_message_event_received),
        )
        .route(
            &format!("{prefix}{ACTIVITY_INSTANCES_PATH}"),
            get(super::process_instances_query::list_activity_instances),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCES_QUERY_PATH}"),
            post(super::process_instances_query::query_process_instances),
        )
        .route(
            &format!("{prefix}{EXECUTIONS_QUERY_PATH}"),
            post(super::process_instances_query::query_executions),
        )
        .route(
            &format!("{prefix}{ACTIVITY_INSTANCES_QUERY_PATH}"),
            post(super::process_instances_query::query_activity_instances),
        )
        .route(
            &format!("{prefix}{VARIABLE_INSTANCES_QUERY_PATH}"),
            post(super::process_instances_query::query_variable_instances),
        )
        .route(
            &format!("{prefix}{VARIABLE_INSTANCES_PATH}"),
            get(super::process_instances_query::list_variable_instances),
        )
        .route(
            &format!("{prefix}{VARIABLE_INSTANCE_DATA_PATH}"),
            get(super::process_instances_query::get_variable_instance_data),
        )
        .layer(Extension(content_service))
}

// ---------------------------------------------------------------------------
// Process-instance attachments — Rust REST extension
//
// Java exposes processInstanceId attachment operations only via TaskService
// (createAttachment / getProcessInstanceAttachments). There is no BPMN REST
// collection under /runtime/process-instances/:id/attachments. These routes
// are an owned Rust extension that reuses the same AttachmentResponse shape
// and content headers as task attachments, and always enforces process
// association on id lookups to prevent cross-process leakage.
// ---------------------------------------------------------------------------

fn load_runtime_process_instance(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Result<ProcessInstance, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_runtime_store()
        .find_process_instance(process_instance_id, &mut session)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Process instance '{}' was not found",
                process_instance_id
            ))
        })
}

fn load_historic_or_runtime_process_instance_id(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Result<String, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    if engine
        .get_runtime_store()
        .find_process_instance(process_instance_id, &mut session)
        .is_some()
    {
        return Ok(process_instance_id.to_string());
    }
    if engine
        .get_runtime_store()
        .get_historic_process_instance(process_instance_id, &mut session)
        .is_some()
    {
        return Ok(process_instance_id.to_string());
    }
    Err(ApiError::NotFound(format!(
        "Process instance '{}' was not found",
        process_instance_id
    )))
}

fn user_id_from_basic_auth(headers: &HeaderMap) -> Option<String> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())?;
    let encoded = auth_header.strip_prefix("Basic ")?;
    let decoded = BASE64_STANDARD.decode(encoded).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let user_id = decoded.split_once(':')?.0.trim();
    if user_id.is_empty() {
        None
    } else {
        Some(user_id.to_string())
    }
}

/// Create attachment on a runtime process instance (write path).
pub(crate) async fn create_process_attachment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path(process_instance_id): Path<String>,
    headers: HeaderMap,
    request: Request,
) -> Result<(StatusCode, Json<super::attachments::AttachmentResponse>), ApiError> {
    let process_instance = load_runtime_process_instance(&engine, &process_instance_id)?;
    if process_instance.is_suspended {
        return Err(ApiError::InternalServerError(format!(
            "It is not allowed to add an attachment to a suspended process instance '{}'",
            process_instance_id
        )));
    }

    let user_id = user_id_from_basic_auth(&headers);
    let input = if super::attachments::is_multipart_request(&headers) {
        super::attachments::parse_multipart_attachment(request).await?
    } else {
        super::attachments::parse_json_attachment(request).await?
    };

    let item = content_service.create_process_attachment(
        process_instance_id.clone(),
        None,
        input.name,
        input.description,
        input.attachment_type,
        input.external_url,
        input.content,
        user_id,
    )?;

    Ok((
        StatusCode::CREATED,
        Json(super::attachments::process_attachment_response_from_record(
            &process_instance_id,
            item,
        )),
    ))
}

/// List attachments for a process instance (readable after completion via historic PI).
pub(crate) async fn list_process_attachments(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path(process_instance_id): Path<String>,
) -> Result<Json<Vec<super::attachments::AttachmentResponse>>, ApiError> {
    let process_instance_id =
        load_historic_or_runtime_process_instance_id(&engine, &process_instance_id)?;
    let attachments = content_service
        .list_process_attachments(&process_instance_id)?
        .into_iter()
        .map(|item| {
            super::attachments::process_attachment_response_from_record(
                &process_instance_id,
                item,
            )
        })
        .collect();
    Ok(Json(attachments))
}

/// Get one attachment scoped to the process instance (no cross-process leakage).
pub(crate) async fn get_process_attachment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path((process_instance_id, attachment_id)): Path<(String, String)>,
) -> Result<Json<super::attachments::AttachmentResponse>, ApiError> {
    let process_instance_id =
        load_historic_or_runtime_process_instance_id(&engine, &process_instance_id)?;
    let item =
        content_service.get_process_attachment(&process_instance_id, &attachment_id)?;
    Ok(Json(
        super::attachments::process_attachment_response_from_record(&process_instance_id, item),
    ))
}

/// Binary content for a process-scoped attachment.
pub(crate) async fn get_process_attachment_content(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path((process_instance_id, attachment_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let process_instance_id =
        load_historic_or_runtime_process_instance_id(&engine, &process_instance_id)?;
    let content =
        content_service.get_process_attachment_content(&process_instance_id, &attachment_id)?;
    let content_type = super::attachments::content_type_for_attachment(
        content
            .attachment_type
            .as_deref()
            .or(content.mime_type.as_deref()),
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(content.bytes))
        .map_err(|err| ApiError::InternalServerError(err.to_string()))
}

/// Delete requires a runtime process instance; 204 empty body.
pub(crate) async fn delete_process_attachment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path((process_instance_id, attachment_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let _process_instance = load_runtime_process_instance(&engine, &process_instance_id)?;
    let user_id = user_id_from_basic_auth(&headers);
    content_service.delete_process_attachment(
        &process_instance_id,
        &attachment_id,
        user_id.as_deref(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(crate) struct StartRequest {
    #[serde(rename = "processDefinitionId")]
    process_definition_id: Option<String>,
    #[serde(rename = "processDefinitionKey")]
    process_definition_key: Option<String>,
    // Java ProcessInstanceCollectionResource.java:381-382: `message` starts a
    // process instance via a message start event subscription.
    message: Option<String>,
    #[serde(rename = "businessKey")]
    business_key: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "returnVariables", default)]
    return_variables: bool,
    #[serde(default)]
    variables: Vec<VariableRequest>,
    // Java ProcessInstanceCollectionResource.java:360-368,402-403.
    #[serde(rename = "transientVariables", default)]
    transient_variables: Vec<VariableRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkDeleteProcessInstancesRequest {
    pub action: Option<String>,
    #[serde(default)]
    pub instance_ids: Vec<String>,
    pub delete_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProcessInstanceRequest {
    pub delete_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProcessInstanceQuery {
    pub delete_reason: Option<String>,
}

#[derive(Debug, Default)]
pub struct UpdateProcessInstanceRequest {
    pub action: Option<String>,
    pub name: Option<Option<String>>,
    pub business_key: Option<Option<String>>,
    pub business_status: Option<Option<String>>,
    pub callback_id: Option<Option<String>>,
    pub callback_type: Option<Option<String>>,
    pub reference_id: Option<Option<String>>,
    pub reference_type: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InjectActivityRequest {
    pub(crate) injection_type: Option<String>,
    #[serde(alias = "activityId")]
    pub(crate) id: Option<String>,
    #[serde(alias = "activityName")]
    pub(crate) name: Option<String>,
    pub(crate) assignee: Option<String>,
    pub(crate) task_id: Option<String>,
    pub(crate) process_definition_id: Option<String>,
    #[serde(default = "default_join_parallel_activities_on_complete")]
    pub(crate) join_parallel_activities_on_complete: bool,
    #[serde(default)]
    pub(crate) variables: Vec<VariableRequest>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessInstanceMigrationRequest {
    pub(crate) migration_document: Option<MigrationDocument>,
    pub(crate) migrate_to_process_definition_id: Option<String>,
    pub(crate) to_process_definition_id: Option<String>,
    #[serde(default, alias = "activityMappings")]
    pub(crate) activity_migration_mappings: Vec<ActivityMigrationMapping>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MigrationDocument {
    pub(crate) migrate_to_process_definition_id: Option<String>,
    pub(crate) to_process_definition_id: Option<String>,
    #[serde(default, alias = "activityMappings")]
    pub(crate) activity_migration_mappings: Vec<ActivityMigrationMapping>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityMigrationMapping {
    pub(crate) from_activity_ids: Option<Vec<String>>,
    pub(crate) from_activity_id: Option<String>,
    pub(crate) to_activity_ids: Option<Vec<String>>,
    pub(crate) to_activity_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MigrationValidationResultResponse {
    pub(crate) valid: bool,
    pub(crate) validation_messages: Vec<String>,
    pub(crate) source_process_definition_id: String,
    pub(crate) target_process_definition_id: String,
    pub(crate) migration_type: String,
}

/// Parsed change-state request. Built by `parse_change_activity_state_request`
/// (manual JSON parsing so P67 shapes can stay exclusive of cancel+start normalize).
#[derive(Debug, Default)]
pub(crate) struct ChangeActivityStateRequest {
    pub(crate) cancel_activity_ids: Vec<String>,
    pub(crate) start_activity_ids: Vec<String>,
    pub(crate) move_activity_id_to: HashMap<String, String>,
    pub(crate) move_activity_ids_to_single_activity_id: HashMap<String, Vec<String>>,
    pub(crate) move_single_activity_id_to_activity_ids: HashMap<String, Vec<String>>,
    /// P67 / Java `ChangeActivityStateBuilder#moveExecutionToActivityId`
    /// (`ChangeActivityStateBuilderImpl.java:53-61`).
    ///
    /// True execution-level move: preserves execution id / locals. Not normalized
    /// into cancelActivityIds+startActivityIds.
    ///
    /// Accepted shapes:
    /// - string activity id (execution endpoint uses path id; process-instance
    ///   endpoint requires companion `executionId`)
    /// - object `{ "executionId": "...", "activityId": "..." }`
    pub(crate) move_execution_to_activity_id: Option<MoveExecutionToActivityIdSpec>,
    /// Optional companion when `moveExecutionToActivityId` is a bare activity id
    /// on the process-instance change-state endpoint.
    pub(crate) execution_id: Option<String>,
    /// P67 / Java `ChangeActivityStateBuilder#enableEventSubProcessStartEvent`
    /// (`ChangeActivityStateBuilderImpl.java:177-182`).
    pub(crate) enable_event_sub_process_start_event: Option<String>,
}

/// Parsed `moveExecutionToActivityId` body field.
#[derive(Debug, Clone)]
pub(crate) enum MoveExecutionToActivityIdSpec {
    /// Bare activity id string.
    ActivityId(String),
    /// Explicit execution + activity pair.
    Pair {
        execution_id: String,
        activity_id: String,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInstanceModificationCommand {
    #[serde(default)]
    pub cancel_activity_ids: Vec<String>,
    #[serde(default)]
    pub start_before_activity_ids: Vec<String>,
    #[serde(default)]
    pub start_after_activity_ids: Vec<String>,
    #[serde(default)]
    pub move_activity_id_to: HashMap<String, String>,
}

#[derive(Debug)]
struct ParsedInjectActivityRequest {
    payload: InjectActivityRequest,
    variables_present: bool,
}

fn default_join_parallel_activities_on_complete() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub(crate) struct ProcessInstanceResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "processDefinitionId")]
    pub process_definition_id: String,
    #[serde(rename = "businessKey")]
    pub business_key: Option<String>,
    #[serde(rename = "businessStatus", skip_serializing_if = "Option::is_none")]
    pub business_status: Option<String>,
    #[serde(rename = "callbackId", skip_serializing_if = "Option::is_none")]
    pub callback_id: Option<String>,
    #[serde(rename = "callbackType", skip_serializing_if = "Option::is_none")]
    pub callback_type: Option<String>,
    #[serde(rename = "referenceId", skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
    #[serde(rename = "referenceType", skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<String>,
    #[serde(rename = "isEnded")]
    pub is_ended: bool,
    #[serde(rename = "isSuspended")]
    pub is_suspended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<Vec<RestVariableResponse>>,
}

fn to_process_instance_response_with_variables(
    instance: ProcessInstance,
    variables: Option<Vec<RestVariableResponse>>,
) -> ProcessInstanceResponse {
    ProcessInstanceResponse {
        id: instance.id,
        name: instance.name,
        process_definition_id: instance.process_definition_id,
        business_key: instance.business_key,
        business_status: instance.business_status,
        callback_id: instance.callback_id,
        callback_type: instance.callback_type,
        reference_id: instance.reference_id,
        reference_type: instance.reference_type,
        is_ended: instance.is_ended,
        is_suspended: instance.is_suspended,
        variables,
    }
}

pub(crate) fn to_process_instance_response(instance: ProcessInstance) -> ProcessInstanceResponse {
    to_process_instance_response_with_variables(instance, None)
}

pub(crate) async fn start(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<ProcessInstanceResponse>, ApiError> {
    let start_user_id = user_id_from_basic_auth(&headers);
    let mut payload: StartRequest =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let process_definition_id = payload
        .process_definition_id
        .take()
        .filter(|value| !value.trim().is_empty());
    let process_definition_key = payload
        .process_definition_key
        .take()
        .filter(|value| !value.trim().is_empty());
    let message = payload
        .message
        .take()
        .filter(|value| !value.trim().is_empty());
    let tenant_id = payload
        .tenant_id
        .take()
        .filter(|value| !value.trim().is_empty());

    // Java ProcessInstanceCollectionResource.java:320-322
    if process_definition_id.is_none() && process_definition_key.is_none() && message.is_none() {
        return Err(ApiError::BadRequest(
            "Either processDefinitionId, processDefinitionKey or message is required.".to_string(),
        ));
    }
    // Java ProcessInstanceCollectionResource.java:324-328
    let params_set = u8::from(process_definition_id.is_some())
        + u8::from(process_definition_key.is_some())
        + u8::from(message.is_some());
    if params_set > 1 {
        return Err(ApiError::BadRequest(
            "Only one of processDefinitionId, processDefinitionKey or message should be set."
                .to_string(),
        ));
    }
    // Java ProcessInstanceCollectionResource.java:330-335
    if process_definition_id.is_some() && tenant_id.is_some() {
        return Err(ApiError::BadRequest(
            "TenantId can only be used with either processDefinitionKey or message.".to_string(),
        ));
    }

    let mut response_variables = Vec::with_capacity(payload.variables.len());
    let mut start_variables = Vec::with_capacity(payload.variables.len());
    for variable in payload.variables {
        ensure_json_supported(&variable)?;
        let name = variable
            .name
            .ok_or_else(|| ApiError::BadRequest("Variable name is required".to_string()))?;
        let value = convert_explicit_variable_value(
            Some(&name),
            variable.variable_type.as_deref(),
            &variable.value,
        )?;
        if payload.return_variables {
            response_variables.push(to_rest_variable_response(name.clone(), value.clone()));
        }
        start_variables.push((name, value));
    }
    // Java ProcessInstanceCollectionResource.java:360-368: transient variables
    // follow the same structure as `variables` but are never persisted, so they
    // are excluded from the returnVariables response (Java :416-423 reads back
    // persistent runtime variables only).
    let mut transient_variables = Vec::with_capacity(payload.transient_variables.len());
    for variable in payload.transient_variables {
        ensure_json_supported(&variable)?;
        let name = variable
            .name
            .ok_or_else(|| ApiError::BadRequest("Variable name is required".to_string()))?;
        let value = convert_explicit_variable_value(
            Some(&name),
            variable.variable_type.as_deref(),
            &variable.value,
        )?;
        transient_variables.push((name, value));
    }

    let pi = if let Some(message) = message {
        // Java ProcessInstanceCollectionResource.java:381-382: messageName start.
        // A missing subscription maps to 400 (Java :439-441 converts
        // FlowableObjectNotFoundException into FlowableIllegalArgumentException).
        let mut cmd = TriggerProcessStartByEventCmd::new(EventSubscriptionKind::Message, message)
            .with_variables(start_variables.iter().cloned().collect())
            .with_transient_variables(transient_variables.iter().cloned().collect());
        if let Some(bk) = payload.business_key {
            cmd = cmd.with_business_key(bk);
        }
        if let Some(tenant_id) = tenant_id {
            cmd = cmd.with_tenant_id(tenant_id);
        }
        if let Some(start_user_id) = start_user_id.as_deref() {
            cmd = cmd.with_start_user_id(start_user_id.to_string());
        }
        engine
            .get_command_executor()
            .execute(&cmd)
            .map_err(|error| match error {
                flowable_engine::error::FlowableError::NotFound(msg) => {
                    ApiError::BadRequest(msg)
                }
                other => other.into(),
            })?
    } else {
        let mut builder = engine
            .get_runtime_service()
            .create_process_instance_builder();
        if let Some(process_definition_id) = process_definition_id {
            builder = builder.process_definition_id(process_definition_id);
        } else if let Some(process_definition_key) = process_definition_key {
            let process_definition = engine
                .get_repository_service()
                .latest_process_definition_by_key(&process_definition_key, tenant_id.as_deref())?
                .ok_or_else(|| {
                    let tenant_suffix = tenant_id
                        .as_deref()
                        .map(|tenant_id| format!(" and tenantId '{}'", tenant_id))
                        .unwrap_or_default();
                    ApiError::NotFound(format!(
                        "Process definition with key '{}'{} was not found",
                        process_definition_key, tenant_suffix
                    ))
                })?;
            builder = builder
                .process_definition_id(process_definition.id)
                .process_definition_key(process_definition.key);
            if let Some(tenant_id) = tenant_id {
                builder = builder.tenant_id(tenant_id);
            }
        }

        if let Some(bk) = payload.business_key {
            builder = builder.business_key(bk);
        }
        if let Some(start_user_id) = start_user_id {
            builder = builder.start_user_id(start_user_id);
        }

        for (name, value) in start_variables {
            builder = builder.variable(name, value);
        }
        // Java ProcessInstanceCollectionResource.java:402-403.
        for (name, value) in transient_variables {
            builder = builder.transient_variable(name, value);
        }

        engine.get_runtime_service().start_process_instance(builder)?
    };

    let variables = if payload.return_variables {
        response_variables.sort_by(|left, right| left.name.cmp(&right.name));
        Some(response_variables)
    } else {
        None
    };

    Ok(Json(to_process_instance_response_with_variables(
        pi, variables,
    )))
}

pub(crate) async fn get_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<Json<ProcessInstanceResponse>, ApiError> {
    let instance = engine
        .get_runtime_store()
        .db_store()
        .find_by_id::<ProcessInstance>("process_instances", &process_instance_id)
        .unwrap()
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Process instance '{}' was not found",
                process_instance_id
            ))
        })?;

    Ok(Json(to_process_instance_response(instance)))
}

pub(crate) async fn delete_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    Query(query): Query<DeleteProcessInstanceQuery>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let payload = if body.trim().is_empty() {
        DeleteProcessInstanceRequest::default()
    } else {
        serde_json::from_str(&body).map_err(|err| ApiError::BadRequest(err.to_string()))?
    };
    let delete_reason = payload.delete_reason.or(query.delete_reason);

    engine
        .get_runtime_service()
        .delete_process_instance(process_instance_id, delete_reason)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn update_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    body: String,
) -> Result<Json<ProcessInstanceResponse>, ApiError> {
    if body.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "Update process instance request body is required".to_string(),
        ));
    }
    let payload = parse_update_process_instance_request(&body)?;
    let UpdateProcessInstanceRequest {
        action,
        name,
        business_key,
        business_status,
        callback_id,
        callback_type,
        reference_id,
        reference_type,
    } = payload;
    let updates = ProcessInstanceUpdate {
        name,
        business_key,
        business_status,
        callback_id,
        callback_type,
        reference_id,
        reference_type,
    };
    let instance = match action.as_deref() {
        Some("suspend") => {
            ensure_process_instance_suspend_action_allowed(&engine, &process_instance_id, true)?;
            engine
                .get_runtime_service()
                .suspend_process_instance(process_instance_id, updates)?
        }
        Some("activate") => {
            ensure_process_instance_suspend_action_allowed(&engine, &process_instance_id, false)?;
            engine
                .get_runtime_service()
                .activate_process_instance(process_instance_id, updates)?
        }
        Some("update") | None if updates.has_updates() => engine
            .get_runtime_service()
            .update_process_instance(process_instance_id, updates)?,
        Some("update") | None => {
            return Err(ApiError::BadRequest(
                "At least one process instance field is required for update action".to_string(),
            ));
        }
        Some(action) => {
            return Err(ApiError::BadRequest(format!("Illegal action: '{action}'.")));
        }
    };

    Ok(Json(to_process_instance_response(instance)))
}

fn ensure_process_instance_suspend_action_allowed(
    engine: &ProcessEngine,
    process_instance_id: &str,
    suspend: bool,
) -> Result<(), ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let process_instance = engine
        .get_runtime_store()
        .find_process_instance(process_instance_id, &mut session)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Process instance '{}' was not found",
                process_instance_id
            ))
        })?;
    if suspend && process_instance.is_suspended {
        return Err(ApiError::Conflict(format!(
            "Process instance with id '{}' is already suspended.",
            process_instance.id
        )));
    }
    if !suspend && !process_instance.is_suspended {
        return Err(ApiError::Conflict(format!(
            "Process instance with id '{}' is already active.",
            process_instance.id
        )));
    }
    Ok(())
}

fn parse_update_process_instance_request(
    body: &str,
) -> Result<UpdateProcessInstanceRequest, ApiError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::BadRequest("Request body must be a JSON object".to_string()))?;
    for field in object.keys() {
        if !matches!(
            field.as_str(),
            "action"
                | "name"
                | "businessKey"
                | "businessStatus"
                | "callbackId"
                | "callbackType"
                | "referenceId"
                | "referenceType"
        ) {
            return Err(ApiError::BadRequest(format!(
                "Unsupported process instance update field '{field}'"
            )));
        }
    }
    let action = match object.get("action") {
        Some(value) if value.is_null() => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| ApiError::BadRequest("action must be a string".to_string()))?
                .to_string(),
        ),
        None => None,
    };

    Ok(UpdateProcessInstanceRequest {
        action,
        name: optional_string_update(object.get("name").cloned(), "name")?,
        business_key: optional_string_update(object.get("businessKey").cloned(), "businessKey")?,
        business_status: optional_string_update(
            object.get("businessStatus").cloned(),
            "businessStatus",
        )?,
        callback_id: optional_string_update(object.get("callbackId").cloned(), "callbackId")?,
        callback_type: optional_string_update(object.get("callbackType").cloned(), "callbackType")?,
        reference_id: optional_string_update(object.get("referenceId").cloned(), "referenceId")?,
        reference_type: optional_string_update(
            object.get("referenceType").cloned(),
            "referenceType",
        )?,
    })
}

fn optional_string_update(
    value: Option<serde_json::Value>,
    field_name: &str,
) -> Result<Option<Option<String>>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(Some(None));
    }
    value
        .as_str()
        .map(|text| Some(Some(text.to_string())))
        .ok_or_else(|| ApiError::BadRequest(format!("{field_name} must be a string or null")))
}

pub(crate) async fn evaluate_process_instance_conditions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let variables = if body.trim().is_empty() {
        HashMap::new()
    } else {
        variable_requests_to_map(parse_variable_requests(&body)?)?
    };

    engine
        .get_runtime_service()
        .evaluate_conditional_events(process_instance_id, variables)?;
    Ok(StatusCode::OK)
}

pub(crate) async fn change_process_instance_state(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let payload = parse_change_activity_state_request(&body)?;
    let runtime = engine.get_runtime_service();

    // P67 exclusive shapes — dispatch to engine true-move / enable, never
    // cancel+start normalize. Java: ChangeActivityStateBuilderImpl.java:53-61,
    // :177-182; ProcessInstanceResource.changeActivityState for cancel/start path.
    if let Some(start_event_id) = payload.enable_event_sub_process_start_event.as_ref() {
        runtime.enable_event_subprocess_start_event(
            process_instance_id,
            start_event_id.clone(),
        )?;
        return Ok(StatusCode::OK);
    }
    if let Some(spec) = payload.move_execution_to_activity_id.as_ref() {
        let (execution_id, activity_id) =
            resolve_move_execution_spec(spec, payload.execution_id.as_deref(), None)?;
        runtime.move_execution_to_activity_id(execution_id, activity_id)?;
        return Ok(StatusCode::OK);
    }

    if payload.cancel_activity_ids.is_empty() {
        return Err(ApiError::BadRequest(
            "cancelActivityIds must contain at least one activity id".to_string(),
        ));
    }

    runtime.change_process_instance_activity_state(
        process_instance_id,
        payload.cancel_activity_ids,
        payload.start_activity_ids,
    )?;
    Ok(StatusCode::OK)
}

pub(crate) async fn change_execution_state(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let payload = parse_change_activity_state_request(&body)?;
    let runtime = engine.get_runtime_service();

    // P67: true execution-level move on the execution change-state endpoint.
    // Path execution id is authoritative; body executionId (if any) must match.
    if let Some(start_event_id) = payload.enable_event_sub_process_start_event.as_ref() {
        // Resolve process instance from the path execution, then enable.
        let process_instance_id = resolve_process_instance_id_for_execution(
            engine.as_ref(),
            &execution_id,
        )?;
        runtime.enable_event_subprocess_start_event(
            process_instance_id,
            start_event_id.clone(),
        )?;
        return Ok(StatusCode::OK);
    }
    if let Some(spec) = payload.move_execution_to_activity_id.as_ref() {
        let (resolved_execution_id, activity_id) =
            resolve_move_execution_spec(spec, payload.execution_id.as_deref(), Some(&execution_id))?;
        runtime.move_execution_to_activity_id(resolved_execution_id, activity_id)?;
        return Ok(StatusCode::OK);
    }

    runtime.change_execution_activity_state(
        execution_id,
        payload.cancel_activity_ids,
        payload.start_activity_ids,
    )?;
    Ok(StatusCode::OK)
}

/// Resolve process instance id for an execution (P67 enable on execution endpoint).
fn resolve_process_instance_id_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
) -> Result<String, ApiError> {
    let store = engine.get_runtime_store();
    let mut session = store
        .create_session()
        .map_err(|e| ApiError::InternalServerError(e.to_string()))?;
    let execution = store
        .find_execution(execution_id, &mut session)
        .ok_or_else(|| ApiError::NotFound(format!("Execution '{execution_id}' was not found")))?;
    if execution.is_ended {
        return Err(ApiError::NotFound(format!(
            "Execution '{execution_id}' was not found"
        )));
    }
    execution.process_instance_id.ok_or_else(|| {
        ApiError::BadRequest(format!(
            "Execution '{execution_id}' is not attached to a process instance"
        ))
    })
}

/// Resolve `(execution_id, activity_id)` from a moveExecutionToActivityId payload.
///
/// `path_execution_id` is set on the execution change-state endpoint.
fn resolve_move_execution_spec(
    spec: &MoveExecutionToActivityIdSpec,
    companion_execution_id: Option<&str>,
    path_execution_id: Option<&str>,
) -> Result<(String, String), ApiError> {
    match spec {
        MoveExecutionToActivityIdSpec::ActivityId(activity_id) => {
            let activity_id = non_blank_activity_id(activity_id, "moveExecutionToActivityId")?;
            let execution_id = path_execution_id
                .map(|s| s.to_string())
                .or_else(|| companion_execution_id.map(|s| s.to_string()))
                .ok_or_else(|| {
                    ApiError::BadRequest(
                        "moveExecutionToActivityId requires executionId when used on the process-instance change-state endpoint"
                            .to_string(),
                    )
                })?;
            let execution_id = non_blank_activity_id(&execution_id, "executionId")?;
            Ok((execution_id, activity_id))
        }
        MoveExecutionToActivityIdSpec::Pair {
            execution_id,
            activity_id,
        } => {
            let activity_id = non_blank_activity_id(activity_id, "moveExecutionToActivityId.activityId")?;
            let execution_id =
                non_blank_activity_id(execution_id, "moveExecutionToActivityId.executionId")?;
            if let Some(path_id) = path_execution_id
                && path_id != execution_id
            {
                return Err(ApiError::BadRequest(format!(
                    "moveExecutionToActivityId.executionId '{execution_id}' does not match path execution id '{path_id}'"
                )));
            }
            Ok((execution_id, activity_id))
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivateActivityRequest {
    pub(crate) activity_id: Option<String>,
}

pub(crate) async fn execution_activate_activity(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    Json(payload): Json<ActivateActivityRequest>,
) -> Result<StatusCode, ApiError> {
    let activity_id = payload
        .activity_id
        .ok_or_else(|| ApiError::BadRequest("activityId is required".to_string()))?;

    engine
        .get_runtime_service()
        .activate_execution_activity(execution_id, activity_id)?;

    Ok(StatusCode::OK)
}

pub(crate) async fn modify_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    Json(command): Json<ProcessInstanceModificationCommand>,
) -> Result<StatusCode, ApiError> {
    if command.cancel_activity_ids.is_empty()
        && command.start_before_activity_ids.is_empty()
        && command.start_after_activity_ids.is_empty()
    {
        return Err(ApiError::BadRequest(
            "At least one of cancelActivityIds, startBeforeActivityIds, or startAfterActivityIds must be specified"
                .to_string(),
        ));
    }

    let runtime_service = engine.get_runtime_service();

    // Combine start activities
    let mut start_activity_ids = command.start_before_activity_ids.clone();
    start_activity_ids.extend(command.start_after_activity_ids.clone());

    // Use change_process_instance_activity_state for the modification
    if !command.cancel_activity_ids.is_empty() || !start_activity_ids.is_empty() {
        runtime_service.change_process_instance_activity_state(
            process_instance_id,
            command.cancel_activity_ids,
            start_activity_ids,
        )?;
    }

    Ok(StatusCode::OK)
}

fn parse_change_activity_state_request(body: &str) -> Result<ChangeActivityStateRequest, ApiError> {
    if body.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "Change state request body is required".to_string(),
        ));
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let object = value.as_object().ok_or_else(|| {
        ApiError::BadRequest("Change state request body must be a JSON object".to_string())
    })?;
    let mut request = ChangeActivityStateRequest {
        cancel_activity_ids: parse_optional_string_array(object, "cancelActivityIds")?
            .unwrap_or_default(),
        start_activity_ids: parse_optional_string_array(object, "startActivityIds")?
            .unwrap_or_default(),
        move_activity_id_to: parse_move_activity_id_to(object.get("moveActivityIdTo"))?,
        move_activity_ids_to_single_activity_id: parse_move_many_to_single(
            object.get("moveActivityIdsToSingleActivityId"),
        )?,
        move_single_activity_id_to_activity_ids: parse_move_single_to_many(
            object.get("moveSingleActivityIdToActivityIds"),
        )?,
        move_execution_to_activity_id: parse_move_execution_to_activity_id(
            object.get("moveExecutionToActivityId"),
        )?,
        execution_id: object
            .get("executionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        enable_event_sub_process_start_event: object
            .get("enableEventSubProcessStartEvent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    };
    // P67 exclusive shapes: do not cancel+start-normalize, and do not require
    // cancel/start when only true-move or enable is supplied.
    let p67_exclusive = request.move_execution_to_activity_id.is_some()
        || request
            .enable_event_sub_process_start_event
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
    if p67_exclusive {
        let legacy_shape = !request.cancel_activity_ids.is_empty()
            || !request.start_activity_ids.is_empty()
            || !request.move_activity_id_to.is_empty()
            || !request.move_activity_ids_to_single_activity_id.is_empty()
            || !request.move_single_activity_id_to_activity_ids.is_empty();
        if legacy_shape {
            return Err(ApiError::BadRequest(
                "moveExecutionToActivityId / enableEventSubProcessStartEvent cannot be combined with cancelActivityIds, startActivityIds, or moveActivity* shapes"
                    .to_string(),
            ));
        }
        if request.move_execution_to_activity_id.is_some()
            && request
                .enable_event_sub_process_start_event
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
        {
            return Err(ApiError::BadRequest(
                "Use either moveExecutionToActivityId or enableEventSubProcessStartEvent, not both"
                    .to_string(),
            ));
        }
        if let Some(start_event_id) = request.enable_event_sub_process_start_event.as_ref() {
            let trimmed = start_event_id.trim();
            if trimmed.is_empty() {
                return Err(ApiError::BadRequest(
                    "enableEventSubProcessStartEvent must not be blank".to_string(),
                ));
            }
            request.enable_event_sub_process_start_event = Some(trimmed.to_string());
        }
        return Ok(request);
    }
    normalize_change_activity_state_request(&mut request)?;
    if request.cancel_activity_ids.is_empty() && request.start_activity_ids.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one of cancelActivityIds or startActivityIds must contain an activity id"
                .to_string(),
        ));
    }
    Ok(request)
}

/// Parse `moveExecutionToActivityId`: string activity id, or object with
/// `executionId` + `activityId` (Java builder arity: executionId, activityId).
fn parse_move_execution_to_activity_id(
    value: Option<&serde_json::Value>,
) -> Result<Option<MoveExecutionToActivityIdSpec>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if let Some(activity_id) = value.as_str() {
        return Ok(Some(MoveExecutionToActivityIdSpec::ActivityId(
            activity_id.to_string(),
        )));
    }
    let object = value.as_object().ok_or_else(|| {
        ApiError::BadRequest(
            "moveExecutionToActivityId must be a string activity id or an object with executionId and activityId"
                .to_string(),
        )
    })?;
    let execution_id = object
        .get("executionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ApiError::BadRequest(
                "moveExecutionToActivityId.executionId is required".to_string(),
            )
        })?
        .to_string();
    let activity_id = object
        .get("activityId")
        .and_then(|v| v.as_str())
        .or_else(|| {
            // Tolerate the same alias set used by moveActivity shapes.
            object
                .get("targetActivityId")
                .and_then(|v| v.as_str())
                .or_else(|| object.get("toActivityId").and_then(|v| v.as_str()))
        })
        .ok_or_else(|| {
            ApiError::BadRequest(
                "moveExecutionToActivityId.activityId is required".to_string(),
            )
        })?
        .to_string();
    Ok(Some(MoveExecutionToActivityIdSpec::Pair {
        execution_id,
        activity_id,
    }))
}

fn parse_optional_string_array(
    object: &serde_json::Map<String, serde_json::Value>,
    field_name: &str,
) -> Result<Option<Vec<String>>, ApiError> {
    let Some(value) = object.get(field_name) else {
        return Ok(None);
    };
    let array = value
        .as_array()
        .ok_or_else(|| ApiError::BadRequest(format!("{field_name} must be an array")))?;
    array
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(|text| text.to_string())
                .ok_or_else(|| ApiError::BadRequest(format!("{field_name} must contain strings")))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn parse_move_activity_id_to(
    value: Option<&serde_json::Value>,
) -> Result<HashMap<String, String>, ApiError> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let entries = single_or_array_entries(value, "moveActivityIdTo")?;
    let mut moves = HashMap::with_capacity(entries.len());
    for entry in entries {
        if let Some((source, target)) = parse_string_map_entry(entry, "moveActivityIdTo")? {
            moves.insert(source, target);
            continue;
        }
        let source = string_field(
            entry,
            &[
                "sourceActivityId",
                "fromActivityId",
                "currentActivityId",
                "cancelActivityId",
                "activityId",
                "source",
            ],
            "moveActivityIdTo source",
        )?;
        let target = string_field(
            entry,
            &[
                "targetActivityId",
                "toActivityId",
                "newActivityId",
                "startActivityId",
                "target",
            ],
            "moveActivityIdTo target",
        )?;
        moves.insert(source, target);
    }
    Ok(moves)
}

fn parse_move_many_to_single(
    value: Option<&serde_json::Value>,
) -> Result<HashMap<String, Vec<String>>, ApiError> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let entries = single_or_array_entries(value, "moveActivityIdsToSingleActivityId")?;
    let mut moves = HashMap::with_capacity(entries.len());
    for entry in entries {
        if let Some((target, sources)) =
            parse_string_array_map_entry(entry, "moveActivityIdsToSingleActivityId")?
        {
            moves.insert(target, sources);
            continue;
        }
        let sources = string_array_field(
            entry,
            &[
                "sourceActivityIds",
                "fromActivityIds",
                "currentActivityIds",
                "cancelActivityIds",
                "activityIds",
                "sources",
            ],
            "moveActivityIdsToSingleActivityId sources",
        )?;
        let target = string_field(
            entry,
            &[
                "targetActivityId",
                "toActivityId",
                "newActivityId",
                "startActivityId",
                "target",
            ],
            "moveActivityIdsToSingleActivityId target",
        )?;
        moves.insert(target, sources);
    }
    Ok(moves)
}

fn parse_move_single_to_many(
    value: Option<&serde_json::Value>,
) -> Result<HashMap<String, Vec<String>>, ApiError> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let entries = single_or_array_entries(value, "moveSingleActivityIdToActivityIds")?;
    let mut moves = HashMap::with_capacity(entries.len());
    for entry in entries {
        if let Some((source, targets)) =
            parse_string_array_map_entry(entry, "moveSingleActivityIdToActivityIds")?
        {
            moves.insert(source, targets);
            continue;
        }
        let source = string_field(
            entry,
            &[
                "sourceActivityId",
                "fromActivityId",
                "currentActivityId",
                "cancelActivityId",
                "activityId",
                "source",
            ],
            "moveSingleActivityIdToActivityIds source",
        )?;
        let targets = string_array_field(
            entry,
            &[
                "targetActivityIds",
                "toActivityIds",
                "newActivityIds",
                "startActivityIds",
                "activityIds",
                "targets",
            ],
            "moveSingleActivityIdToActivityIds targets",
        )?;
        moves.insert(source, targets);
    }
    Ok(moves)
}

fn single_or_array_entries<'a>(
    value: &'a serde_json::Value,
    field_name: &str,
) -> Result<Vec<&'a serde_json::Value>, ApiError> {
    if let Some(entries) = value.as_array() {
        if entries.is_empty() {
            return Err(ApiError::BadRequest(format!(
                "{field_name} must contain at least one move"
            )));
        }
        Ok(entries.iter().collect())
    } else if value.is_object() {
        Ok(vec![value])
    } else {
        Err(ApiError::BadRequest(format!(
            "{field_name} must be an object or an array of objects"
        )))
    }
}

fn parse_string_map_entry(
    value: &serde_json::Value,
    field_name: &str,
) -> Result<Option<(String, String)>, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::BadRequest(format!("{field_name} entries must be objects")))?;
    if object.len() != 1 {
        return Ok(None);
    }
    let (source, target) = object.iter().next().unwrap();
    if is_move_descriptor_field(source) {
        return Ok(None);
    }
    let Some(target) = target.as_str() else {
        return Ok(None);
    };
    Ok(Some((source.clone(), target.to_string())))
}

fn parse_string_array_map_entry(
    value: &serde_json::Value,
    field_name: &str,
) -> Result<Option<(String, Vec<String>)>, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::BadRequest(format!("{field_name} entries must be objects")))?;
    if object.len() != 1 {
        return Ok(None);
    }
    let (key, array_value) = object.iter().next().unwrap();
    if is_move_descriptor_field(key) {
        return Ok(None);
    }
    let Some(array) = array_value.as_array() else {
        return Ok(None);
    };
    let values = array
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(|text| text.to_string())
                .ok_or_else(|| ApiError::BadRequest(format!("{field_name} values must be strings")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some((key.clone(), values)))
}

fn is_move_descriptor_field(name: &str) -> bool {
    matches!(
        name,
        "sourceActivityId"
            | "sourceActivityIds"
            | "fromActivityId"
            | "fromActivityIds"
            | "currentActivityId"
            | "currentActivityIds"
            | "cancelActivityId"
            | "cancelActivityIds"
            | "activityId"
            | "activityIds"
            | "source"
            | "sources"
            | "targetActivityId"
            | "targetActivityIds"
            | "toActivityId"
            | "toActivityIds"
            | "newActivityId"
            | "newActivityIds"
            | "startActivityId"
            | "startActivityIds"
            | "target"
            | "targets"
    )
}

fn string_field(
    value: &serde_json::Value,
    names: &[&str],
    field_name: &str,
) -> Result<String, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::BadRequest(format!("{field_name} entry must be an object")))?;
    for name in names {
        if let Some(value) = object.get(*name) {
            return value
                .as_str()
                .map(|text| text.to_string())
                .ok_or_else(|| ApiError::BadRequest(format!("{field_name} must be a string")));
        }
    }
    Err(ApiError::BadRequest(format!("{field_name} is required")))
}

fn string_array_field(
    value: &serde_json::Value,
    names: &[&str],
    field_name: &str,
) -> Result<Vec<String>, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::BadRequest(format!("{field_name} entry must be an object")))?;
    for name in names {
        if let Some(value) = object.get(*name) {
            let array = value
                .as_array()
                .ok_or_else(|| ApiError::BadRequest(format!("{field_name} must be an array")))?;
            return array
                .iter()
                .map(|entry| {
                    entry.as_str().map(|text| text.to_string()).ok_or_else(|| {
                        ApiError::BadRequest(format!("{field_name} must contain strings"))
                    })
                })
                .collect();
        }
    }
    Err(ApiError::BadRequest(format!("{field_name} are required")))
}

fn normalize_change_activity_state_request(
    request: &mut ChangeActivityStateRequest,
) -> Result<(), ApiError> {
    let direct_shape_used =
        !request.cancel_activity_ids.is_empty() || !request.start_activity_ids.is_empty();
    let move_activity_shape_count = usize::from(!request.move_activity_id_to.is_empty())
        + usize::from(!request.move_activity_ids_to_single_activity_id.is_empty())
        + usize::from(!request.move_single_activity_id_to_activity_ids.is_empty());
    if direct_shape_used && move_activity_shape_count > 0 {
        return Err(ApiError::BadRequest(
            "Use either cancelActivityIds/startActivityIds or one moveActivity shape, not both"
                .to_string(),
        ));
    }
    if move_activity_shape_count > 1 {
        return Err(ApiError::BadRequest(
            "Only one moveActivity change-state shape can be used per request".to_string(),
        ));
    }

    if !request.move_activity_id_to.is_empty() {
        if request.move_activity_id_to.len() != 1 {
            return Err(ApiError::BadRequest(
                "moveActivityIdTo supports exactly one source and target activity".to_string(),
            ));
        }
        let (source, target) = request.move_activity_id_to.iter().next().unwrap();
        request.cancel_activity_ids =
            vec![non_blank_activity_id(source, "moveActivityIdTo source")?];
        request.start_activity_ids =
            vec![non_blank_activity_id(target, "moveActivityIdTo target")?];
    } else if !request.move_activity_ids_to_single_activity_id.is_empty() {
        if request.move_activity_ids_to_single_activity_id.len() != 1 {
            return Err(ApiError::BadRequest(
                "moveActivityIdsToSingleActivityId supports exactly one target activity"
                    .to_string(),
            ));
        }
        let (target, sources) = request
            .move_activity_ids_to_single_activity_id
            .iter()
            .next()
            .unwrap();
        if sources.is_empty() {
            return Err(ApiError::BadRequest(
                "moveActivityIdsToSingleActivityId must contain at least one source activity id"
                    .to_string(),
            ));
        }
        request.cancel_activity_ids = sources
            .iter()
            .map(|source| non_blank_activity_id(source, "moveActivityIdsToSingleActivityId source"))
            .collect::<Result<Vec<_>, _>>()?;
        request.start_activity_ids = vec![non_blank_activity_id(
            target,
            "moveActivityIdsToSingleActivityId target",
        )?];
    } else if !request.move_single_activity_id_to_activity_ids.is_empty() {
        if request.move_single_activity_id_to_activity_ids.len() != 1 {
            return Err(ApiError::BadRequest(
                "moveSingleActivityIdToActivityIds supports exactly one source activity"
                    .to_string(),
            ));
        }
        let (source, targets) = request
            .move_single_activity_id_to_activity_ids
            .iter()
            .next()
            .unwrap();
        if targets.is_empty() {
            return Err(ApiError::BadRequest(
                "moveSingleActivityIdToActivityIds must contain at least one target activity id"
                    .to_string(),
            ));
        }
        request.cancel_activity_ids = vec![non_blank_activity_id(
            source,
            "moveSingleActivityIdToActivityIds source",
        )?];
        request.start_activity_ids = targets
            .iter()
            .map(|target| non_blank_activity_id(target, "moveSingleActivityIdToActivityIds target"))
            .collect::<Result<Vec<_>, _>>()?;
    }

    Ok(())
}

fn non_blank_activity_id(value: &str, field_name: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "{field_name} must not be blank"
        )));
    }
    Ok(trimmed.to_string())
}

pub(crate) async fn bulk_delete_process_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let payload: BulkDeleteProcessInstancesRequest =
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
            "At least one process instance id is required.".to_string(),
        ));
    }

    engine
        .get_runtime_service()
        .bulk_delete_process_instances(payload.instance_ids, payload.delete_reason)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn inject_process_instance_activity(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let parsed = parse_inject_activity_request(&body)?;
    let payload = parsed.payload;
    let mut session = engine.get_runtime_store().create_session().unwrap();
    if engine
        .get_runtime_store()
        .find_process_instance(&process_instance_id, &mut session)
        .is_none()
    {
        return Err(ApiError::NotFound(format!(
            "Process instance '{}' was not found",
            process_instance_id
        )));
    }

    let _process_definition_id = payload.process_definition_id;
    let _join_parallel_activities_on_complete = payload.join_parallel_activities_on_complete;
    let variables = variable_requests_to_map(payload.variables)?;
    if parsed.variables_present
        && payload
            .injection_type
            .as_deref()
            .is_some_and(|injection_type| !is_task_injection_type(injection_type))
    {
        return Err(ApiError::BadRequest(
            "variables are only supported for task activity injection".to_string(),
        ));
    }

    let result = match payload.injection_type.as_deref() {
        Some(injection_type) if is_task_injection_type(injection_type) => {
            let task_id = payload
                .id
                .filter(|id| !id.trim().is_empty())
                .or_else(|| payload.task_id.filter(|id| !id.trim().is_empty()))
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let name = payload.name.unwrap_or_else(|| task_id.clone());
            let assignee = payload.assignee.clone();
            engine.get_runtime_service().inject_user_task(
                process_instance_id.clone(),
                task_id.clone(),
                name,
                assignee.clone(),
            )?;
            if assignee.is_some() {
                engine.get_task_service().update_task_by_id(
                    task_id,
                    TaskUpdate {
                        assignee: Some(assignee),
                        ..TaskUpdate::default()
                    },
                )?;
            }
            Ok(())
        }
        Some(injection_type) if injection_type.eq_ignore_ascii_case("subprocess") => {
            let activity_id = payload
                .id
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    ApiError::BadRequest(
                        "id is required for subprocess activity injection".to_string(),
                    )
                })?;
            engine
                .get_runtime_service()
                .inject_subprocess_activity(process_instance_id.clone(), activity_id)?;
            Ok(())
        }
        Some(injection_type) if injection_type.eq_ignore_ascii_case("startBefore") => {
            let activity_id = payload
                .id
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    ApiError::BadRequest(
                        "id is required for startBefore activity injection".to_string(),
                    )
                })?;
            let cancel_activity_ids =
                active_activity_ids_for_process_instance(&engine, &process_instance_id)?;
            engine
                .get_runtime_service()
                .change_process_instance_activity_state(
                    process_instance_id.clone(),
                    cancel_activity_ids,
                    vec![activity_id],
                )?;
            Ok(())
        }
        Some(injection_type) if injection_type.eq_ignore_ascii_case("startAfter") => {
            let activity_id = payload
                .id
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    ApiError::BadRequest(
                        "id is required for startAfter activity injection".to_string(),
                    )
                })?;
            engine
                .get_runtime_service()
                .inject_start_after_activity(process_instance_id.clone(), activity_id)?;
            Ok(())
        }
        Some(injection_type) => Err(ApiError::BadRequest(format!(
            "injection type is not supported {injection_type}"
        ))),
        None => Err(ApiError::BadRequest(
            "injectionType is required".to_string(),
        )),
    };
    result?;
    if parsed.variables_present {
        let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
        for (name, value) in variables {
            engine
                .get_variable_service()
                .set_variable(execution.id.clone(), name, value)?;
        }
    }
    Ok(StatusCode::OK)
}

fn is_task_injection_type(injection_type: &str) -> bool {
    injection_type.eq_ignore_ascii_case("task") || injection_type.eq_ignore_ascii_case("userTask")
}

fn parse_inject_activity_request(body: &str) -> Result<ParsedInjectActivityRequest, ApiError> {
    if body.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "Inject activity request body is required".to_string(),
        ));
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let object = value.as_object().ok_or_else(|| {
        ApiError::BadRequest("Inject activity request body must be a JSON object".to_string())
    })?;
    let variables_present = object.contains_key("variables");
    let payload: InjectActivityRequest =
        serde_json::from_value(value).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(ParsedInjectActivityRequest {
        payload,
        variables_present,
    })
}

fn active_activity_ids_for_process_instance(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Result<Vec<String>, ApiError> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut activity_ids = store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && !execution.is_ended
                && !execution.is_suspended
        })
        .filter_map(|execution| execution.activity_id)
        .filter(|activity_id| !activity_id.trim().is_empty())
        .collect::<Vec<_>>();
    activity_ids.sort();
    activity_ids.dedup();
    if activity_ids.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "Process instance '{}' has no active activity to move for startBefore injection",
            process_instance_id
        )));
    }
    Ok(activity_ids)
}

pub(crate) async fn validate_process_instance_migration(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    body: String,
) -> Result<Json<MigrationValidationResultResponse>, ApiError> {
    let payload = parse_migration_request(&body)?;
    let result =
        validate_process_instance_migration_request(&engine, &process_instance_id, &payload)?;
    Ok(Json(result))
}

pub(crate) async fn migrate_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let payload = parse_migration_request(&body)?;
    let result =
        validate_process_instance_migration_request(&engine, &process_instance_id, &payload)?;
    if !result.valid {
        return Err(ApiError::BadRequest(result.validation_messages.join("; ")));
    }
    migrate_process_instance_if_safe(
        &engine,
        &process_instance_id,
        &result.target_process_definition_id,
        runtime_activity_migration_mappings(&payload)?,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn parse_migration_request(
    body: &str,
) -> Result<ProcessInstanceMigrationRequest, ApiError> {
    if body.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "Migration request body is required".to_string(),
        ));
    }
    serde_json::from_str(body).map_err(|error| ApiError::BadRequest(error.to_string()))
}

pub(crate) fn target_process_definition_id(
    request: &ProcessInstanceMigrationRequest,
) -> Option<&str> {
    request
        .migration_document
        .as_ref()
        .and_then(|document| {
            document
                .migrate_to_process_definition_id
                .as_deref()
                .or(document.to_process_definition_id.as_deref())
        })
        .or(request.migrate_to_process_definition_id.as_deref())
        .or(request.to_process_definition_id.as_deref())
}

pub(crate) fn runtime_activity_migration_mappings(
    request: &ProcessInstanceMigrationRequest,
) -> Result<Vec<RuntimeActivityMigrationMapping>, ApiError> {
    let mappings = request
        .migration_document
        .as_ref()
        .map(|document| document.activity_migration_mappings.as_slice())
        .filter(|mappings| !mappings.is_empty())
        .unwrap_or(request.activity_migration_mappings.as_slice());

    mappings
        .iter()
        .try_fold(Vec::new(), |mut runtime_mappings, mapping| {
            let from_activity_ids = one_or_more_activity_ids(
                mapping.from_activity_id.as_deref(),
                mapping.from_activity_ids.as_deref(),
                "fromActivityId",
                "fromActivityIds",
            )?;
            let to_activity_ids = one_or_more_activity_ids(
                mapping.to_activity_id.as_deref(),
                mapping.to_activity_ids.as_deref(),
                "toActivityId",
                "toActivityIds",
            )?;
            if from_activity_ids.len() > 1 && to_activity_ids.len() > 1 {
                return Err(ApiError::BadRequest(
                    "Only single-to-many or many-to-single activity migration mappings are supported"
                        .to_string(),
                ));
            }
            runtime_mappings.extend(from_activity_ids.into_iter().map(|from_activity_id| {
                RuntimeActivityMigrationMapping {
                    from_activity_id,
                    to_activity_ids: to_activity_ids.clone(),
                }
            }));
            Ok(runtime_mappings)
        })
}

fn one_or_more_activity_ids(
    scalar: Option<&str>,
    list: Option<&[String]>,
    scalar_name: &str,
    list_name: &str,
) -> Result<Vec<String>, ApiError> {
    let list_values = list.unwrap_or_default();
    match (scalar.filter(|value| !value.trim().is_empty()), list_values) {
        (Some(_), values) if !values.is_empty() => Err(ApiError::BadRequest(format!(
            "Only one of {scalar_name} or {list_name} can be provided"
        ))),
        (Some(value), _) => Ok(vec![value.to_string()]),
        (None, []) => Err(ApiError::BadRequest(format!("{scalar_name} is required"))),
        (None, values) => {
            let activity_ids = values
                .iter()
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .collect::<Vec<_>>();
            if activity_ids.is_empty() {
                Err(ApiError::BadRequest(format!("{scalar_name} is required")))
            } else {
                Ok(activity_ids)
            }
        }
    }
}

pub(crate) fn validate_process_instance_migration_request(
    engine: &ProcessEngine,
    process_instance_id: &str,
    request: &ProcessInstanceMigrationRequest,
) -> Result<MigrationValidationResultResponse, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let instance = engine
        .get_runtime_store()
        .find_process_instance(process_instance_id, &mut session)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Process instance '{}' was not found",
                process_instance_id
            ))
        })?;
    engine
        .get_repository_service()
        .get_process_definition(&instance.process_definition_id)?;
    let target_definition_id = target_process_definition_id(request)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            ApiError::BadRequest(
                "migrateToProcessDefinitionId is required in the request or migrationDocument"
                    .to_string(),
            )
        })?
        .to_string();
    engine
        .get_repository_service()
        .get_process_definition(&target_definition_id)?;

    let mut validation_messages = Vec::new();
    let mappings = match runtime_activity_migration_mappings(request) {
        Ok(mappings) => mappings,
        Err(ApiError::BadRequest(message)) => {
            validation_messages.push(message);
            Vec::new()
        }
        Err(error) => return Err(error),
    };
    validation_messages.extend(validate_runtime_wait_state_migration(
        engine,
        &instance.id,
        &target_definition_id,
        &mappings,
    )?);

    if instance.process_definition_id != target_definition_id
        && process_instance_has_active_runtime_state(engine, &instance.id)
        && !validation_messages.is_empty()
    {
        validation_messages.push(format!(
            "Process instance '{}' has active runtime state that cannot be migrated by this Rust runtime migration subset",
            instance.id
        ));
    }

    Ok(MigrationValidationResultResponse {
        valid: validation_messages.is_empty(),
        validation_messages,
        source_process_definition_id: instance.process_definition_id,
        target_process_definition_id: target_definition_id,
        migration_type: "processInstanceMigration".to_string(),
    })
}

pub(crate) fn migrate_process_instance_if_safe(
    engine: &ProcessEngine,
    process_instance_id: &str,
    target_process_definition_id: &str,
    activity_migration_mappings: Vec<RuntimeActivityMigrationMapping>,
) -> Result<(), ApiError> {
    engine.get_runtime_service().migrate_process_instance(
        process_instance_id.to_string(),
        target_process_definition_id.to_string(),
        activity_migration_mappings,
    )?;
    Ok(())
}

fn validate_runtime_wait_state_migration(
    engine: &ProcessEngine,
    process_instance_id: &str,
    target_process_definition_id: &str,
    activity_migration_mappings: &[RuntimeActivityMigrationMapping],
) -> Result<Vec<String>, ApiError> {
    let mapping_lookup = activity_migration_mappings
        .iter()
        .map(|mapping| {
            (
                mapping.from_activity_id.as_str(),
                mapping.to_activity_ids.as_slice(),
            )
        })
        .collect::<HashMap<_, _>>();
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let active_executions = store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && execution.activity_id.is_some()
                && !execution.is_ended
                && !execution.is_suspended
        })
        .collect::<Vec<_>>();

    let target_model = engine
        .get_repository_service()
        .get_bpmn_model(target_process_definition_id)?;

    let mut validation_messages = Vec::new();
    for execution in active_executions {
        let Some(source_activity_id) = execution.activity_id.as_deref() else {
            continue;
        };
        let target_activity_ids = mapping_lookup
            .get(source_activity_id)
            .map(|activity_ids| activity_ids.iter().map(String::as_str).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![source_activity_id]);

        if store
            .find_task_by_execution_id(&execution.id, &mut session)
            .is_none()
        {
            validation_messages.push(format!(
                "Execution '{}' is not waiting at a user task and cannot be migrated by this Rust runtime migration subset",
                execution.id
            ));
            continue;
        }

        let Some(main_process) = target_model.main_process.as_ref() else {
            validation_messages.push(format!(
                "Target process definition '{}' has no main process",
                target_process_definition_id
            ));
            continue;
        };
        for target_activity_id in target_activity_ids {
            let target_is_user_task =
                main_process
                    .flow_elements
                    .iter()
                    .any(|flow_element| match flow_element {
                        flowable_bpmn_model::model::FlowElementEnum::UserTask(user_task) => {
                            user_task
                                .task
                                .activity
                                .flow_node
                                .flow_element
                                .base_element
                                .id
                                .as_deref()
                                == Some(target_activity_id)
                        }
                        _ => false,
                    });
            if !target_is_user_task {
                validation_messages.push(format!(
                    "Target activity '{}' must be a userTask for runtime wait-state migration",
                    target_activity_id
                ));
            }
        }
    }

    Ok(validation_messages)
}

pub(crate) fn process_instance_has_active_runtime_state(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> bool {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    if store
        .snapshot_executions(&mut session)
        .into_values()
        .any(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && !execution.is_ended
        })
    {
        return true;
    }
    if store
        .find_tasks_by_process_instance_id(process_instance_id, &mut session)
        .into_iter()
        .any(|task| !task.is_completed)
    {
        return true;
    }
    if !store
        .find_event_wait_states_by_process_instance_id(process_instance_id, &mut session)
        .is_empty()
    {
        return true;
    }
    if !store
        .find_boundary_event_states_by_process_instance_id(process_instance_id, &mut session)
        .is_empty()
    {
        return true;
    }
    if !store
        .find_timer_job_states_by_process_instance_id(process_instance_id, &mut session)
        .is_empty()
    {
        return true;
    }
    if !store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut session,
        )
        .is_empty()
    {
        return true;
    }
    if !store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut session,
        )
        .is_empty()
    {
        return true;
    }
    !store
        .find_compensation_subscriptions_by_process_instance_id(process_instance_id, &mut session)
        .is_empty()
}

#[derive(Debug, Deserialize)]
pub(crate) struct VariableRequest {
    pub(crate) name: Option<String>,
    #[serde(rename = "type")]
    pub(crate) variable_type: Option<String>,
    pub(crate) value: serde_json::Value,
    /// Optional `local`/`global` scope label (Java `RestVariable.scope`). The
    /// task-, execution-, and process-instance-variable routes consume it.
    pub(crate) scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RestVariableResponse {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) variable_type: String,
    pub(crate) value: serde_json::Value,
    pub(crate) scope: String,
}

const BINARY_VARIABLE_MARKER_FIELD: &str = "__flowableRustRestVariableData";
const BINARY_VARIABLE_TYPE_FIELD: &str = "type";
const BINARY_VARIABLE_DATA_FIELD: &str = "data";

struct BinaryVariableData {
    variable_type: String,
    bytes: Vec<u8>,
}

pub(crate) fn to_rest_variable_response(
    name: String,
    value: serde_json::Value,
) -> RestVariableResponse {
    if let Some(variable_type) = variable_data_type(&value) {
        return RestVariableResponse {
            variable_type,
            name,
            value: serde_json::Value::Null,
            scope: "local".to_string(),
        };
    }
    RestVariableResponse {
        variable_type: rest_variable_type(&value).to_string(),
        name,
        value,
        scope: "local".to_string(),
    }
}

/// Response shape of the process-instance SINGLE-variable endpoints. Java
/// `ProcessInstanceVariableResource.constructRestVariable` overrides the scope
/// to `null` (variable type `VARIABLE_PROCESS`), so unlike the execution
/// endpoints these responses carry no scope label. The collection endpoints
/// still label every variable `local`
/// (`ProcessInstanceVariableCollectionResource.addLocalVariables`).
#[derive(Debug, Serialize)]
pub(crate) struct ProcessVariableResponse {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) variable_type: String,
    pub(crate) value: serde_json::Value,
    pub(crate) scope: Option<String>,
}

fn process_variable_response(name: String, value: serde_json::Value) -> ProcessVariableResponse {
    let RestVariableResponse {
        name,
        variable_type,
        value,
        ..
    } = to_rest_variable_response(name, value);
    ProcessVariableResponse {
        name,
        variable_type,
        value,
        scope: None,
    }
}

pub(crate) fn variable_data_type(value: &serde_json::Value) -> Option<String> {
    encoded_variable_data_type(value, &["binary", "bytes", "serializable"])
}

fn encoded_variable_data_type(
    value: &serde_json::Value,
    supported_types: &[&str],
) -> Option<String> {
    let object = value.as_object()?;
    if object
        .get(BINARY_VARIABLE_MARKER_FIELD)
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return None;
    }
    object
        .get(BINARY_VARIABLE_TYPE_FIELD)
        .and_then(serde_json::Value::as_str)
        .filter(|variable_type| supported_types.contains(variable_type))
        .map(str::to_string)
}

pub(crate) fn encode_variable_data(variable_type: &str, bytes: &[u8]) -> serde_json::Value {
    serde_json::json!({
        BINARY_VARIABLE_MARKER_FIELD: true,
        BINARY_VARIABLE_TYPE_FIELD: variable_type,
        BINARY_VARIABLE_DATA_FIELD: BASE64_STANDARD.encode(bytes),
    })
}

pub(crate) fn encode_binary_variable(variable_type: &str, bytes: &[u8]) -> serde_json::Value {
    encode_variable_data(variable_type, bytes)
}

fn decode_variable_data(value: &serde_json::Value) -> Result<Option<BinaryVariableData>, ApiError> {
    let Some(variable_type) = variable_data_type(value) else {
        return Ok(None);
    };
    let data = value
        .as_object()
        .and_then(|object| object.get(BINARY_VARIABLE_DATA_FIELD))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ApiError::InternalServerError("Binary variable data marker is malformed".to_string())
        })?;
    let bytes = BASE64_STANDARD
        .decode(data)
        .map_err(|err| ApiError::InternalServerError(err.to_string()))?;
    Ok(Some(BinaryVariableData {
        variable_type,
        bytes,
    }))
}

pub(crate) fn storage_value_for_data_backed_variable_request(
    request: &VariableRequest,
) -> Result<serde_json::Value, ApiError> {
    storage_value_for_variable_request_with_serializable(request, true)
}

fn storage_value_for_variable_request_with_serializable(
    request: &VariableRequest,
    allow_serializable: bool,
) -> Result<serde_json::Value, ApiError> {
    let Some(variable_type) = request.variable_type.as_deref() else {
        return Ok(request.value.clone());
    };
    match variable_type.to_ascii_lowercase().as_str() {
        "binary" | "bytes" => {
            if !request.value.is_null() {
                return Err(ApiError::BadRequest(format!(
                    "Variable type '{}' metadata must use null value; write bytes with the variable data endpoint",
                    variable_type
                )));
            }
            Ok(encode_binary_variable(
                &variable_type.to_ascii_lowercase(),
                &[],
            ))
        }
        "serializable" => {
            if !allow_serializable {
                return Err(ApiError::BadRequest(
                    "Variable type 'serializable' is not supported by this REST JSON subset"
                        .to_string(),
                ));
            }
            if !request.value.is_null() {
                return Err(ApiError::BadRequest(format!(
                    "Variable type '{}' metadata must use null value; write object data with the variable data endpoint",
                    variable_type
                )));
            }
            Ok(encode_variable_data("serializable", &[]))
        }
        _ => convert_explicit_variable_value(
            request.name.as_deref(),
            request.variable_type.as_deref(),
            &request.value,
        ),
    }
}

pub(crate) fn parse_variable_requests(body: &str) -> Result<Vec<VariableRequest>, ApiError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| ApiError::BadRequest(error.to_string()))?;
    parse_variable_requests_value(value)
}

pub(crate) fn parse_variable_requests_value(
    value: serde_json::Value,
) -> Result<Vec<VariableRequest>, ApiError> {
    if value.is_array() {
        serde_json::from_value(value).map_err(|error| ApiError::BadRequest(error.to_string()))
    } else {
        serde_json::from_value(value)
            .map(|request| vec![request])
            .map_err(|error| ApiError::BadRequest(error.to_string()))
    }
}

pub(crate) fn variable_requests_to_map(
    requests: Vec<VariableRequest>,
) -> Result<HashMap<String, serde_json::Value>, ApiError> {
    let mut variables = HashMap::with_capacity(requests.len());
    for request in requests {
        ensure_json_supported(&request)?;
        let name = request
            .name
            .ok_or_else(|| ApiError::BadRequest("Variable name is required".to_string()))?;
        let value = convert_explicit_variable_value(
            Some(&name),
            request.variable_type.as_deref(),
            &request.value,
        )?;
        variables.insert(name, value);
    }
    Ok(variables)
}

fn ensure_json_supported(request: &VariableRequest) -> Result<(), ApiError> {
    let Some(variable_type) = request.variable_type.as_deref() else {
        return Ok(());
    };
    if matches!(
        variable_type.to_ascii_lowercase().as_str(),
        "binary" | "bytes" | "serializable"
    ) {
        return Err(ApiError::BadRequest(format!(
            "Variable type '{}' is not supported by JSON variable endpoints",
            variable_type
        )));
    }
    Ok(())
}

fn find_current_execution_for_process_instance(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Result<Execution, ApiError> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    if let Some(execution) = store
        .find_execution(process_instance_id, &mut session)
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && !execution.is_ended
        })
    {
        return Ok(execution);
    }

    let mut executions: Vec<Execution> = engine
        .get_runtime_store()
        .db_store()
        .find_all::<Execution>("executions")
        .unwrap()
        .into_iter()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && !execution.is_ended
        })
        .collect();

    executions.sort_by(|left, right| left.id.cmp(&right.id));
    match executions.len() {
        1 => Ok(executions.remove(0)),
        0 => Err(ApiError::NotFound(format!(
            "Process instance '{}' has no active execution for variables",
            process_instance_id
        ))),
        _ => executions
            .into_iter()
            .find(|execution| execution.is_scope && execution.parent_id.is_none())
            .ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "Process instance '{}' has multiple active executions; use /runtime/executions/{{executionId}}/variables",
                    process_instance_id
                ))
            }),
    }
}

fn find_execution(engine: &ProcessEngine, execution_id: &str) -> Result<Execution, ApiError> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .find_execution(execution_id, &mut session)
        .ok_or_else(|| ApiError::NotFound(format!("Execution '{}' was not found", execution_id)))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ExecutionVariableScopeQuery {
    scope: Option<String>,
}

/// Java parity: `RestVariable.getScopeFromString` — an unknown scope string is a
/// 400 with this exact message, whether it arrives as `?scope=` or in the body.
fn parse_execution_variable_scope(
    scope: Option<&str>,
) -> Result<Option<ExecutionVariableScope>, ApiError> {
    match scope {
        None => Ok(None),
        Some(scope) if scope.eq_ignore_ascii_case("local") => {
            Ok(Some(ExecutionVariableScope::Local))
        }
        Some(scope) if scope.eq_ignore_ascii_case("global") => {
            Ok(Some(ExecutionVariableScope::Global))
        }
        Some(scope) => Err(ApiError::BadRequest(format!(
            "Invalid variable scope: '{scope}'"
        ))),
    }
}

/// Reads the `?scope=` query parameter of an execution-variable request.
fn execution_variable_scope_from_query(
    uri: &axum::http::Uri,
) -> Result<Option<ExecutionVariableScope>, ApiError> {
    let query: ExecutionVariableScopeQuery = crate::common::parse_query(uri)?;
    parse_execution_variable_scope(query.scope.as_deref())
}

/// Resolves the effective write scope: the per-variable body scope wins over the
/// `?scope=` fallback; with neither present Java defaults to the local scope
/// (`BaseExecutionVariableResource.setSimpleVariable`).
fn resolve_execution_write_scope(
    body_scope: Option<&str>,
    query_scope: Option<ExecutionVariableScope>,
) -> Result<ExecutionVariableScope, ApiError> {
    Ok(parse_execution_variable_scope(body_scope)?
        .or(query_scope)
        .unwrap_or(ExecutionVariableScope::Local))
}

fn execution_scope_label(scope: ExecutionVariableScope) -> &'static str {
    match scope {
        ExecutionVariableScope::Local => "local",
        ExecutionVariableScope::Global => "global",
    }
}

fn scoped_execution_variable_response(
    name: String,
    value: serde_json::Value,
    scope: ExecutionVariableScope,
) -> RestVariableResponse {
    let mut response = to_rest_variable_response(name, value);
    response.scope = execution_scope_label(scope).to_string();
    response
}

/// Java `getVariableFromRequestWithoutAccessCheck` 404 message.
fn execution_variable_not_found(execution_id: &str, variable_name: &str) -> ApiError {
    ApiError::NotFound(format!(
        "Execution '{execution_id}' does not have a variable with name: '{variable_name}'."
    ))
}

/// Shared body of the scoped collection read used by the execution- and
/// process-instance-variable endpoints (Java `processVariables`).
pub(crate) fn scoped_variables_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
    scope: Option<ExecutionVariableScope>,
) -> Result<Vec<RestVariableResponse>, ApiError> {
    find_execution(engine, execution_id)?;
    Ok(engine
        .get_variable_service()
        .get_variables_on_scope(execution_id.to_string(), scope)?
        .into_iter()
        .map(|(name, value, scope)| scoped_execution_variable_response(name, value, scope))
        .collect())
}

/// Shared body of the scoped single-variable read (Java
/// `getVariableFromRequestWithoutAccessCheck`).
fn scoped_variable_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
    variable_name: &str,
    scope: Option<ExecutionVariableScope>,
) -> Result<RestVariableResponse, ApiError> {
    let (value, scope) =
        scoped_variable_value_for_execution(engine, execution_id, variable_name, scope)?;
    Ok(scoped_execution_variable_response(
        variable_name.to_string(),
        value,
        scope,
    ))
}

fn scoped_variable_value_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
    variable_name: &str,
    scope: Option<ExecutionVariableScope>,
) -> Result<(serde_json::Value, ExecutionVariableScope), ApiError> {
    find_execution(engine, execution_id)?;
    engine
        .get_variable_service()
        .get_variable_on_scope(execution_id.to_string(), variable_name.to_string(), scope)?
        .ok_or_else(|| execution_variable_not_found(execution_id, variable_name))
}

/// Parses and validates a batch of variable write requests the way Java
/// `createExecutionVariable` does: the list must be non-empty, every variable
/// needs a name, and the whole batch shares one scope. Returns the shared
/// scope and the mutations.
fn resolve_scoped_variable_mutations(
    requests: &[VariableRequest],
    query_scope: Option<ExecutionVariableScope>,
) -> Result<(ExecutionVariableScope, Vec<ExecutionVariableMutation>), ApiError> {
    // Java: an empty variable list is a 400.
    if requests.is_empty() {
        return Err(ApiError::BadRequest(
            "Request did not contain a list of variables to create.".to_string(),
        ));
    }
    let mut shared_scope: Option<ExecutionVariableScope> = None;
    let mut mutations = Vec::with_capacity(requests.len());
    for request in requests {
        let name = request
            .name
            .clone()
            .ok_or_else(|| ApiError::BadRequest("Variable name is required".to_string()))?;
        let scope = resolve_execution_write_scope(request.scope.as_deref(), query_scope)?;
        // Java: all variables in one request must resolve to the same scope.
        match shared_scope {
            None => shared_scope = Some(scope),
            Some(shared) if shared != scope => {
                return Err(ApiError::BadRequest(
                    "Only allowed to update multiple variables in the same scope.".to_string(),
                ));
            }
            _ => {}
        }
        let value = storage_value_for_data_backed_variable_request(request)?;
        mutations.push(ExecutionVariableMutation { name, value });
    }
    let scope = shared_scope.expect("non-empty batch has a shared scope");
    Ok((scope, mutations))
}

/// Shared body of the scoped batch write (Java `createExecutionVariable`), used
/// by the create-only POST and the upsert PUT.
fn mutate_scoped_variables_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
    requests: Vec<VariableRequest>,
    query_scope: Option<ExecutionVariableScope>,
    mode: VariableMutationMode,
) -> Result<Vec<RestVariableResponse>, ApiError> {
    find_execution(engine, execution_id)?;
    let (scope, mutations) = resolve_scoped_variable_mutations(&requests, query_scope)?;
    engine.get_variable_service().mutate_variables_on_scope(
        execution_id.to_string(),
        scope,
        mode,
        mutations.clone(),
    )?;
    Ok(mutations
        .into_iter()
        .map(|mutation| scoped_execution_variable_response(mutation.name, mutation.value, scope))
        .collect())
}

/// Shared body of the scoped async batch write (Java `createExecutionVariable`
/// with `async = true`): the same synchronous validation as the sync write,
/// then a `set-async-variables` job is scheduled instead of writing.
fn mutate_scoped_variables_for_execution_async(
    engine: &ProcessEngine,
    execution_id: &str,
    requests: Vec<VariableRequest>,
    query_scope: Option<ExecutionVariableScope>,
    mode: VariableMutationMode,
) -> Result<(), ApiError> {
    find_execution(engine, execution_id)?;
    let (scope, mutations) = resolve_scoped_variable_mutations(&requests, query_scope)?;
    engine
        .get_variable_service()
        .mutate_variables_on_scope_async(execution_id.to_string(), scope, mode, mutations)?;
    Ok(())
}

/// Parses and validates a single-variable write body the way Java
/// `setSimpleVariable` does: the body's name must match the URL name, and the
/// scope resolves from the body first, then the query. Returns the scope and
/// the storage value.
fn parse_scoped_single_variable_write(
    variable_name: &str,
    body: &str,
    query_scope: Option<ExecutionVariableScope>,
) -> Result<(ExecutionVariableScope, serde_json::Value), ApiError> {
    let request: VariableRequest =
        serde_json::from_str(body).map_err(|error| ApiError::BadRequest(error.to_string()))?;
    if let Some(name) = request.name.as_deref()
        && name != variable_name
    {
        return Err(ApiError::BadRequest(
            "Variable name in the body should be equal to the name used in the requested URL."
                .to_string(),
        ));
    }
    let scope = resolve_execution_write_scope(request.scope.as_deref(), query_scope)?;
    let value = storage_value_for_data_backed_variable_request(&request)?;
    Ok((scope, value))
}

/// Shared body of the scoped single-variable write (Java `setSimpleVariable`).
fn set_scoped_single_variable_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
    variable_name: &str,
    body: &str,
    query_scope: Option<ExecutionVariableScope>,
    mode: VariableMutationMode,
) -> Result<RestVariableResponse, ApiError> {
    find_execution(engine, execution_id)?;
    let (scope, value) = parse_scoped_single_variable_write(variable_name, body, query_scope)?;
    engine.get_variable_service().mutate_variables_on_scope(
        execution_id.to_string(),
        scope,
        mode,
        vec![ExecutionVariableMutation {
            name: variable_name.to_string(),
            value: value.clone(),
        }],
    )?;
    Ok(scoped_execution_variable_response(
        variable_name.to_string(),
        value,
        scope,
    ))
}

/// Shared body of the scoped async single-variable write (Java
/// `setSimpleVariable` with `async = true`, which the `variables-async` PUT
/// endpoints always call update-only): the same synchronous validation as the
/// sync write, then a `set-async-variables` job is scheduled instead of
/// writing.
fn set_scoped_single_variable_for_execution_async(
    engine: &ProcessEngine,
    execution_id: &str,
    variable_name: &str,
    body: &str,
    query_scope: Option<ExecutionVariableScope>,
) -> Result<(), ApiError> {
    find_execution(engine, execution_id)?;
    let (scope, value) = parse_scoped_single_variable_write(variable_name, body, query_scope)?;
    engine
        .get_variable_service()
        .mutate_variables_on_scope_async(
            execution_id.to_string(),
            scope,
            VariableMutationMode::UpdateOnly,
            vec![ExecutionVariableMutation {
                name: variable_name.to_string(),
                value,
            }],
        )?;
    Ok(())
}

pub(crate) fn variables_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
) -> Result<Vec<RestVariableResponse>, ApiError> {
    find_execution(engine, execution_id)?;
    let mut variables = engine
        .get_variable_service()
        .get_variables(execution_id.to_string())?
        .into_iter()
        .map(|(name, value)| to_rest_variable_response(name, value))
        .collect::<Vec<_>>();
    variables.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(variables)
}

pub(crate) fn variable_data_response(value: serde_json::Value) -> Result<Response, ApiError> {
    if let Some(binary) = decode_variable_data(&value)? {
        // Java parity (TaskVariableDataResource/BaseExecutionVariableResource):
        // binary variables stream as application/octet-stream, serializable
        // variables as application/x-java-serialized-object.
        let content_type = if binary.variable_type == "serializable" {
            "application/x-java-serialized-object"
        } else {
            "application/octet-stream"
        };
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(binary.bytes))
            .map_err(|err| ApiError::InternalServerError(err.to_string()));
    }
    Ok(Json(value).into_response())
}

/// Scope-aware variant of [`set_variable_data_for_execution`]: the existing
/// variable is resolved on the requested scope and rewritten there, so a binary
/// update cannot silently move a variable between scopes.
fn set_scoped_variable_data_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
    variable_name: &str,
    scope: Option<ExecutionVariableScope>,
    bytes: Bytes,
) -> Result<(), ApiError> {
    find_execution(engine, execution_id)?;
    let (value, resolved_scope) =
        scoped_variable_value_for_execution(engine, execution_id, variable_name, scope)?;
    let binary = decode_variable_data(&value)?.ok_or_else(|| {
        ApiError::BadRequest(format!(
            "Variable '{}' for execution '{}' is not a binary, bytes, or serializable variable",
            variable_name, execution_id
        ))
    })?;
    engine.get_variable_service().mutate_variables_on_scope(
        execution_id.to_string(),
        resolved_scope,
        VariableMutationMode::UpdateOnly,
        vec![ExecutionVariableMutation {
            name: variable_name.to_string(),
            value: encode_variable_data(&binary.variable_type, &bytes),
        }],
    )?;
    Ok(())
}

pub(crate) fn set_variable_data_for_execution(
    engine: &ProcessEngine,
    execution_id: &str,
    variable_name: &str,
    bytes: Bytes,
) -> Result<(), ApiError> {
    find_execution(engine, execution_id)?;
    let value = engine
        .get_variable_service()
        .get_variable(execution_id.to_string(), variable_name.to_string())?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Variable '{}' was not found for execution '{}'",
                variable_name, execution_id
            ))
        })?;
    let binary = decode_variable_data(&value)?.ok_or_else(|| {
        ApiError::BadRequest(format!(
            "Variable '{}' for execution '{}' is not a binary, bytes, or serializable variable",
            variable_name, execution_id
        ))
    })?;
    engine.get_variable_service().set_variable(
        execution_id.to_string(),
        variable_name.to_string(),
        encode_variable_data(&binary.variable_type, &bytes),
    )?;
    Ok(())
}

/// Java `ProcessInstanceVariableCollectionResource.getVariables`: the process
/// instance row is a root execution, so there is no global scope — an explicit
/// `?scope=global` yields an EMPTY list, while every returned variable is
/// labeled `local` (the `addLocalVariables` override).
pub(crate) async fn list_process_instance_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    uri: axum::http::Uri,
) -> Result<Json<Vec<RestVariableResponse>>, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    Ok(Json(scoped_variables_for_execution(
        &engine,
        &execution.id,
        scope,
    )?))
}

/// Java `createProcessInstanceVariable` (collection POST): create-only on the
/// resolved scope; a GLOBAL write is a 400 because the process instance row
/// has no parent execution.
pub(crate) async fn create_process_instance_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    uri: axum::http::Uri,
    body: String,
) -> Result<(StatusCode, Json<Vec<RestVariableResponse>>), ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    let variables = mutate_scoped_variables_for_execution(
        &engine,
        &execution.id,
        parse_variable_requests(&body)?,
        scope,
        VariableMutationMode::CreateOnly,
    )?;
    Ok((StatusCode::CREATED, Json(variables)))
}

/// Java `createOrUpdateProcessVariable` (collection PUT): the override variant
/// of the POST, upserting instead of conflicting.
pub(crate) async fn update_process_instance_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    uri: axum::http::Uri,
    body: String,
) -> Result<(StatusCode, Json<Vec<RestVariableResponse>>), ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    let variables = mutate_scoped_variables_for_execution(
        &engine,
        &execution.id,
        parse_variable_requests(&body)?,
        scope,
        VariableMutationMode::Upsert,
    )?;
    Ok((StatusCode::CREATED, Json(variables)))
}

/// Java `deleteLocalProcessVariable` (collection DELETE): removes ALL
/// variables of the process instance's own (local) scope.
pub(crate) async fn delete_process_instance_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    engine.get_variable_service().remove_variables_on_scope(
        execution.id,
        ExecutionVariableScope::Local,
        None,
        false,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `ProcessInstanceVariableResource.getVariable`: local value first with
/// no parent fallback on a root execution; the response carries `scope = null`
/// (the `constructRestVariable` override).
pub(crate) async fn get_process_instance_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
) -> Result<Json<ProcessVariableResponse>, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    let (value, _) =
        scoped_variable_value_for_execution(&engine, &execution.id, &variable_name, scope)?;
    Ok(Json(process_variable_response(variable_name, value)))
}

pub(crate) async fn get_process_instance_variable_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
) -> Result<Response, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    let (value, _) =
        scoped_variable_value_for_execution(&engine, &execution.id, &variable_name, scope)?;
    variable_data_response(value)
}

pub(crate) async fn update_process_instance_variable_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    set_scoped_variable_data_for_execution(&engine, &execution.id, &variable_name, scope, body)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `updateProcessInstanceVariable` (single PUT): update-only on the
/// resolved scope. On the root process instance row a GLOBAL update is a 404 —
/// `hasVariableOnScope(GLOBAL)` is always false there
/// (`BaseExecutionVariableResource.setVariable`), never the collection
/// write's 400.
pub(crate) async fn update_process_instance_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
    body: String,
) -> Result<Json<ProcessVariableResponse>, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    let response = set_scoped_single_variable_for_execution(
        &engine,
        &execution.id,
        &variable_name,
        &body,
        scope,
        VariableMutationMode::UpdateOnly,
    )?;
    Ok(Json(process_variable_response(
        response.name,
        response.value,
    )))
}

/// Java `deleteProcessInstanceVariable`: the scope defaults to LOCAL and a
/// variable absent on that scope is a 404 naming the scope.
pub(crate) async fn delete_process_instance_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?.unwrap_or(ExecutionVariableScope::Local);
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    engine.get_variable_service().remove_variables_on_scope(
        execution.id,
        scope,
        Some(vec![variable_name]),
        true,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `createProcessInstanceVariableAsync`: the batch create-only validation
/// runs synchronously; the write itself is scheduled as a `set-async-variables`
/// job, so the 201 only means the job exists. Java parity: the handler carries
/// no `@ResponseStatus`, so the unconditional `response.setStatus(201)` in
/// `BaseVariableCollectionResource.createExecutionVariable` stands
/// (ProcessInstanceVariableCollectionResource.java:178-182 →
/// BaseVariableCollectionResource.java:181).
pub(crate) async fn create_process_instance_variables_async(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    uri: axum::http::Uri,
    body: String,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    mutate_scoped_variables_for_execution_async(
        &engine,
        &execution.id,
        parse_variable_requests(&body)?,
        scope,
        VariableMutationMode::CreateOnly,
    )?;
    Ok(StatusCode::CREATED)
}

/// Java `createOrUpdateProcessInstanceVariableAsync`: the upsert variant of the
/// async batch write; also 201 (no `@ResponseStatus` → base class
/// `setStatus(201)`, ProcessInstanceVariableCollectionResource.java:120-124).
pub(crate) async fn update_process_instance_variables_async(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    uri: axum::http::Uri,
    body: String,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    mutate_scoped_variables_for_execution_async(
        &engine,
        &execution.id,
        parse_variable_requests(&body)?,
        scope,
        VariableMutationMode::Upsert,
    )?;
    Ok(StatusCode::CREATED)
}

/// Java `updateProcessInstanceVariableAsync`: update-only validation runs
/// synchronously; the write is scheduled as a `set-async-variables` job. Java
/// parity: the handler carries `@ResponseStatus(NO_CONTENT)` and
/// `setSimpleVariable` never touches the status — 204
/// (ProcessInstanceVariableResource.java:146-147).
pub(crate) async fn update_process_instance_variable_async(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
    body: String,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let execution = find_current_execution_for_process_instance(&engine, &process_instance_id)?;
    set_scoped_single_variable_for_execution_async(
        &engine,
        &execution.id,
        &variable_name,
        &body,
        scope,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_execution_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    uri: axum::http::Uri,
) -> Result<Json<Vec<RestVariableResponse>>, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    Ok(Json(scoped_variables_for_execution(
        &engine,
        &execution_id,
        scope,
    )?))
}

pub(crate) async fn create_execution_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    uri: axum::http::Uri,
    body: String,
) -> Result<(StatusCode, Json<Vec<RestVariableResponse>>), ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let variables = mutate_scoped_variables_for_execution(
        &engine,
        &execution_id,
        parse_variable_requests(&body)?,
        scope,
        VariableMutationMode::CreateOnly,
    )?;
    Ok((StatusCode::CREATED, Json(variables)))
}

/// Java `createOrUpdateExecutionVariable`: the same batch write as POST but with
/// `override = true`, so an existing variable is updated instead of conflicting.
pub(crate) async fn update_execution_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    uri: axum::http::Uri,
    body: String,
) -> Result<(StatusCode, Json<Vec<RestVariableResponse>>), ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let variables = mutate_scoped_variables_for_execution(
        &engine,
        &execution_id,
        parse_variable_requests(&body)?,
        scope,
        VariableMutationMode::Upsert,
    )?;
    Ok((StatusCode::CREATED, Json(variables)))
}

/// Java `deleteLocalVariables`: removes ALL execution-local variables; the
/// ancestor (global) scope is left untouched.
pub(crate) async fn delete_all_local_execution_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    find_execution(&engine, &execution_id)?;
    engine.get_variable_service().remove_variables_on_scope(
        execution_id,
        ExecutionVariableScope::Local,
        None,
        false,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn get_execution_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((execution_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
) -> Result<Json<RestVariableResponse>, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    Ok(Json(scoped_variable_for_execution(
        &engine,
        &execution_id,
        &variable_name,
        scope,
    )?))
}

pub(crate) async fn get_execution_variable_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((execution_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
) -> Result<Response, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    let (value, _) =
        scoped_variable_value_for_execution(&engine, &execution_id, &variable_name, scope)?;
    variable_data_response(value)
}

pub(crate) async fn update_execution_variable_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((execution_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    set_scoped_variable_data_for_execution(&engine, &execution_id, &variable_name, scope, body)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn update_execution_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((execution_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
    body: String,
) -> Result<Json<RestVariableResponse>, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    Ok(Json(set_scoped_single_variable_for_execution(
        &engine,
        &execution_id,
        &variable_name,
        &body,
        scope,
        VariableMutationMode::UpdateOnly,
    )?))
}

/// Java `deleteVariable`: the scope defaults to LOCAL and a variable absent on
/// that scope is a 404 naming the scope.
pub(crate) async fn delete_execution_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((execution_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?.unwrap_or(ExecutionVariableScope::Local);
    find_execution(&engine, &execution_id)?;
    engine.get_variable_service().remove_variables_on_scope(
        execution_id,
        scope,
        Some(vec![variable_name]),
        true,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `createExecutionVariableAsync`: the batch create-only validation runs
/// synchronously; the write itself is scheduled as a `set-async-variables`
/// job, so the 204 only means the job exists. Java parity: the handler carries
/// `@ResponseStatus(NO_CONTENT)`, which Spring applies after the handler ran,
/// overriding the base class `setStatus(201)`
/// (ExecutionVariableCollectionResource.java:164-165).
pub(crate) async fn create_execution_variables_async(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    uri: axum::http::Uri,
    body: String,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    mutate_scoped_variables_for_execution_async(
        &engine,
        &execution_id,
        parse_variable_requests(&body)?,
        scope,
        VariableMutationMode::CreateOnly,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `createOrUpdateExecutionVariableAsync`: the upsert variant of the
/// async batch write; 204 via `@ResponseStatus(NO_CONTENT)`
/// (ExecutionVariableCollectionResource.java:109-110).
pub(crate) async fn update_execution_variables_async(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    uri: axum::http::Uri,
    body: String,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    mutate_scoped_variables_for_execution_async(
        &engine,
        &execution_id,
        parse_variable_requests(&body)?,
        scope,
        VariableMutationMode::Upsert,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `updateExecutionVariableAsync`: update-only validation runs
/// synchronously; the write is scheduled as a `set-async-variables` job; 204
/// via `@ResponseStatus(NO_CONTENT)` (ExecutionVariableResource.java:152-153).
pub(crate) async fn update_execution_variable_async(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((execution_id, variable_name)): Path<(String, String)>,
    uri: axum::http::Uri,
    body: String,
) -> Result<StatusCode, ApiError> {
    let scope = execution_variable_scope_from_query(&uri)?;
    set_scoped_single_variable_for_execution_async(
        &engine,
        &execution_id,
        &variable_name,
        &body,
        scope,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionSignalEventRequest {
    signal_name: Option<String>,
    #[serde(default)]
    variables: Vec<ExecutionTriggerVariableRequest>,
    /// Accepted from the wire contract for API continuity; the Rust
    /// signal-event handler does not yet act on it.
    #[allow(dead_code)]
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionMessageEventRequest {
    message_name: Option<String>,
    #[serde(default)]
    variables: Vec<ExecutionTriggerVariableRequest>,
    /// Accepted from the wire contract for API continuity; the Rust
    /// message-event handler does not yet act on it.
    #[allow(dead_code)]
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionTriggerVariableRequest {
    name: Option<String>,
    value: serde_json::Value,
    #[serde(rename = "type")]
    _variable_type: Option<String>,
}

fn parse_trigger_variables(
    variables: Vec<ExecutionTriggerVariableRequest>,
) -> Result<Vec<(String, serde_json::Value)>, ApiError> {
    variables
        .into_iter()
        .map(|variable| {
            let name = variable
                .name
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| ApiError::bad_request("Variable name is required."))?;
            Ok((name, variable.value))
        })
        .collect()
}

pub(crate) async fn execution_signal_event_received(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: ExecutionSignalEventRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let signal_name = request
        .signal_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("signalName is required"))?;

    // Java parity: SignalEventReceivedCmd checks execution.isSuspended() for targeted signals
    let execution = find_execution(&engine, &execution_id)?;
    if execution.is_suspended {
        return Err(ApiError::InternalServerError(format!(
            "Cannot throw signal event '{}' because execution '{}' is suspended",
            signal_name, execution_id
        )));
    }

    let variables = parse_trigger_variables(request.variables)?;
    let runtime_store = engine.get_runtime_store();
    let runtime_service = engine.get_runtime_service();
    let mut session = runtime_store.create_session().unwrap();

    let wait_state = runtime_store
        .snapshot_event_wait_states(&mut session)
        .into_values()
        .find(|wait_state| {
            wait_state.execution_id == execution_id
                && wait_state.wait_kind == RuntimeEventWaitKind::SignalIntermediateCatchEvent
                && wait_state
                    .event_subscription
                    .as_ref()
                    .is_some_and(|subscription| {
                        subscription.kind == EventSubscriptionKind::Signal
                            && subscription.event_ref == signal_name
                    })
        });

    if let Some(_wait_state) = wait_state {
        for (name, value) in &variables {
            engine.get_variable_service().set_variable(
                execution_id.clone(),
                name.clone(),
                value.clone(),
            )?;
        }
        runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
            signal_name.to_string(),
            execution_id,
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    let boundary_state = runtime_store
        .snapshot_boundary_event_states(&mut session)
        .into_values()
        .find(|state| {
            state.host_execution_id == execution_id
                && state.event_subscription.kind == EventSubscriptionKind::Signal
                && state.event_subscription.event_ref == signal_name
        });

    if let Some(state) = boundary_state {
        for (name, value) in &variables {
            engine.get_variable_service().set_variable(
                execution_id.clone(),
                name.clone(),
                value.clone(),
            )?;
        }
        runtime_service.trigger_boundary_event_by_signal_ref(
            signal_name.to_string(),
            state.process_instance_id,
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    Err(ApiError::NotFound(format!(
        "Execution '{}' is not waiting for signal '{}'",
        execution_id, signal_name
    )))
}

pub(crate) async fn execution_message_event_received(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: ExecutionMessageEventRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let message_name = request
        .message_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("messageName is required"))?;

    // Java parity: MessageEventReceivedCmd extends NeedsActiveExecutionCmd
    let execution = find_execution(&engine, &execution_id)?;
    if execution.is_suspended {
        return Err(ApiError::InternalServerError(format!(
            "Cannot receive message for a suspended execution '{}'",
            execution_id
        )));
    }

    let variables = parse_trigger_variables(request.variables)?;
    let runtime_store = engine.get_runtime_store();
    let runtime_service = engine.get_runtime_service();
    let mut session = runtime_store.create_session().unwrap();

    let wait_state = runtime_store
        .snapshot_event_wait_states(&mut session)
        .into_values()
        .find(|wait_state| {
            wait_state.execution_id == execution_id
                && matches!(
                    wait_state.wait_kind,
                    RuntimeEventWaitKind::MessageIntermediateCatchEvent
                        | RuntimeEventWaitKind::ReceiveTask
                )
                && wait_state
                    .event_subscription
                    .as_ref()
                    .is_some_and(|subscription| {
                        subscription.kind == EventSubscriptionKind::Message
                            && subscription.event_ref == message_name
                    })
        });

    if let Some(_wait_state) = wait_state {
        for (name, value) in &variables {
            engine.get_variable_service().set_variable(
                execution_id.clone(),
                name.clone(),
                value.clone(),
            )?;
        }
        runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
            message_name.to_string(),
            execution_id,
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    let boundary_state = runtime_store
        .snapshot_boundary_event_states(&mut session)
        .into_values()
        .find(|state| {
            state.host_execution_id == execution_id
                && state.event_subscription.kind == EventSubscriptionKind::Message
                && state.event_subscription.event_ref == message_name
        });

    if let Some(state) = boundary_state {
        for (name, value) in &variables {
            engine.get_variable_service().set_variable(
                execution_id.clone(),
                name.clone(),
                value.clone(),
            )?;
        }
        runtime_service.trigger_boundary_event_by_message_ref(
            message_name.to_string(),
            state.process_instance_id,
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    Err(ApiError::NotFound(format!(
        "Execution '{}' is not waiting for message '{}'",
        execution_id, message_name
    )))
}

/// Java `ExecutionActionRequest` — shared by `ExecutionResource`
/// (PUT /runtime/executions/{executionId}) and `ExecutionCollectionResource`
/// (PUT /runtime/executions).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionActionRequest {
    action: Option<String>,
    signal_name: Option<String>,
    message_name: Option<String>,
    #[serde(default)]
    variables: Vec<ExecutionTriggerVariableRequest>,
    #[serde(default)]
    transient_variables: Vec<ExecutionTriggerVariableRequest>,
}

/// Java `ExecutionResource.performExecutionAction`
/// (ExecutionResource.java:58-110): 404 unknown execution, 400 illegal
/// action or missing signal/message name, 500 when the engine cannot apply
/// the action, then re-fetch — 204 with empty body when the execution
/// finished, otherwise 200 + ExecutionResponse.
pub(crate) async fn perform_execution_action(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let request: ExecutionActionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let execution = find_execution(&engine, &execution_id)?;
    // Java string-concatenates a null action into the error message.
    let action = request.action.as_deref().unwrap_or("null").to_string();

    match action.as_str() {
        // Java `runtimeService.trigger(...)` — TriggerCmd extends
        // NeedsActiveExecutionCmd, so suspension and "not a wait state" are
        // FlowableException → 500.
        "signal" | "trigger" => {
            if execution.is_suspended {
                return Err(ApiError::InternalServerError(format!(
                    "Cannot trigger a suspended execution '{}'",
                    execution_id
                )));
            }
            let variables = parse_trigger_variables(request.variables)?;
            let transient_variables = parse_trigger_variables(request.transient_variables)?;

            let runtime_store = engine.get_runtime_store();
            let wait_state = {
                let mut session = runtime_store.create_session().unwrap();
                runtime_store
                    .snapshot_event_wait_states(&mut session)
                    .into_values()
                    .find(|wait_state| {
                        wait_state.execution_id == execution_id
                            && wait_state.wait_kind == RuntimeEventWaitKind::ReceiveTask
                    })
            };

            let Some(wait_state) = wait_state else {
                // Java TriggerCmd: current activity behaviour is not
                // triggerable → FlowableException (500).
                return Err(ApiError::InternalServerError(format!(
                    "Cannot trigger execution '{}' because it is not waiting in a triggerable state",
                    execution_id
                )));
            };

            if let Some(task_id) = wait_state.task_id.clone() {
                engine
                    .get_task_service()
                    .complete_task_by_id_with_variable_maps(
                        task_id,
                        variables.into_iter().collect(),
                        transient_variables.into_iter().collect(),
                    )?;
            } else if let Some(subscription) = wait_state.event_subscription.clone() {
                for (name, value) in &variables {
                    engine.get_variable_service().set_variable(
                        execution_id.clone(),
                        name.clone(),
                        value.clone(),
                    )?;
                }
                engine
                    .get_runtime_service()
                    .trigger_intermediate_catch_event_by_message_ref_and_execution_id(
                        subscription.event_ref,
                        execution_id.clone(),
                    );
            } else {
                return Err(ApiError::InternalServerError(format!(
                    "Cannot trigger execution '{}' because it is not waiting in a triggerable state",
                    execution_id
                )));
            }
        }
        "signalEventReceived" => {
            // Java only rejects a null signalName (no trailing period).
            let signal_name = request
                .signal_name
                .clone()
                .ok_or_else(|| ApiError::bad_request("Signal name is required"))?;
            perform_targeted_signal_event(&engine, &execution_id, &signal_name, request.variables)?;
        }
        "messageEventReceived" => {
            let message_name = request
                .message_name
                .clone()
                .ok_or_else(|| ApiError::bad_request("Message name is required"))?;
            perform_targeted_message_event(
                &engine,
                &execution_id,
                &message_name,
                request.variables,
            )?;
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "Invalid action: '{}'.",
                other
            )));
        }
    }

    // Java parity: re-fetch — the action may have completed the execution.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    match store.find_execution(&execution_id, &mut session) {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(execution) => Ok((
            StatusCode::OK,
            Json(super::process_instances_query::to_execution_response(
                execution,
            )),
        )
            .into_response()),
    }
}

/// Shared body of the PUT `signalEventReceived` action: same wait-state and
/// boundary lookup as the POST handler, but a miss is a FlowableException
/// (500) in Java `SignalEventReceivedCmd` rather than a 404.
fn perform_targeted_signal_event(
    engine: &ProcessEngine,
    execution_id: &str,
    signal_name: &str,
    variables: Vec<ExecutionTriggerVariableRequest>,
) -> Result<(), ApiError> {
    let execution = find_execution(engine, execution_id)?;
    if execution.is_suspended {
        return Err(ApiError::InternalServerError(format!(
            "Cannot throw signal event '{}' because execution '{}' is suspended",
            signal_name, execution_id
        )));
    }

    let variables = parse_trigger_variables(variables)?;
    let runtime_store = engine.get_runtime_store();
    let runtime_service = engine.get_runtime_service();
    let mut session = runtime_store.create_session().unwrap();

    let wait_state = runtime_store
        .snapshot_event_wait_states(&mut session)
        .into_values()
        .find(|wait_state| {
            wait_state.execution_id == execution_id
                && wait_state.wait_kind == RuntimeEventWaitKind::SignalIntermediateCatchEvent
                && wait_state
                    .event_subscription
                    .as_ref()
                    .is_some_and(|subscription| {
                        subscription.kind == EventSubscriptionKind::Signal
                            && subscription.event_ref == signal_name
                    })
        });

    if wait_state.is_some() {
        for (name, value) in &variables {
            engine.get_variable_service().set_variable(
                execution_id.to_string(),
                name.clone(),
                value.clone(),
            )?;
        }
        runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
            signal_name.to_string(),
            execution_id.to_string(),
        );
        return Ok(());
    }

    let boundary_state = runtime_store
        .snapshot_boundary_event_states(&mut session)
        .into_values()
        .find(|state| {
            state.host_execution_id == execution_id
                && state.event_subscription.kind == EventSubscriptionKind::Signal
                && state.event_subscription.event_ref == signal_name
        });

    if let Some(state) = boundary_state {
        for (name, value) in &variables {
            engine.get_variable_service().set_variable(
                execution_id.to_string(),
                name.clone(),
                value.clone(),
            )?;
        }
        runtime_service.trigger_boundary_event_by_signal_ref(
            signal_name.to_string(),
            state.process_instance_id,
        );
        return Ok(());
    }

    // Java SignalEventReceivedCmd: FlowableException → 500.
    Err(ApiError::InternalServerError(format!(
        "Execution '{}' has not subscribed to a signal event with name '{}'.",
        execution_id, signal_name
    )))
}

/// Shared body of the PUT `messageEventReceived` action; a miss is a
/// FlowableException (500) in Java `MessageEventReceivedCmd`.
fn perform_targeted_message_event(
    engine: &ProcessEngine,
    execution_id: &str,
    message_name: &str,
    variables: Vec<ExecutionTriggerVariableRequest>,
) -> Result<(), ApiError> {
    let execution = find_execution(engine, execution_id)?;
    if execution.is_suspended {
        return Err(ApiError::InternalServerError(format!(
            "Cannot receive message for a suspended execution '{}'",
            execution_id
        )));
    }

    let variables = parse_trigger_variables(variables)?;
    let runtime_store = engine.get_runtime_store();
    let runtime_service = engine.get_runtime_service();
    let mut session = runtime_store.create_session().unwrap();

    let wait_state = runtime_store
        .snapshot_event_wait_states(&mut session)
        .into_values()
        .find(|wait_state| {
            wait_state.execution_id == execution_id
                && matches!(
                    wait_state.wait_kind,
                    RuntimeEventWaitKind::MessageIntermediateCatchEvent
                        | RuntimeEventWaitKind::ReceiveTask
                )
                && wait_state
                    .event_subscription
                    .as_ref()
                    .is_some_and(|subscription| {
                        subscription.kind == EventSubscriptionKind::Message
                            && subscription.event_ref == message_name
                    })
        });

    if let Some(wait_state) = wait_state {
        if let Some(task_id) = wait_state.task_id.clone() {
            engine
                .get_task_service()
                .complete_task_by_id_with_variable_maps(
                    task_id,
                    variables.into_iter().collect(),
                    std::collections::HashMap::new(),
                )?;
        } else {
            for (name, value) in &variables {
                engine.get_variable_service().set_variable(
                    execution_id.to_string(),
                    name.clone(),
                    value.clone(),
                )?;
            }
            runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
                message_name.to_string(),
                execution_id.to_string(),
            );
        }
        return Ok(());
    }

    let boundary_state = runtime_store
        .snapshot_boundary_event_states(&mut session)
        .into_values()
        .find(|state| {
            state.host_execution_id == execution_id
                && state.event_subscription.kind == EventSubscriptionKind::Message
                && state.event_subscription.event_ref == message_name
        });

    if let Some(state) = boundary_state {
        for (name, value) in &variables {
            engine.get_variable_service().set_variable(
                execution_id.to_string(),
                name.clone(),
                value.clone(),
            )?;
        }
        runtime_service.trigger_boundary_event_by_message_ref(
            message_name.to_string(),
            state.process_instance_id,
        );
        return Ok(());
    }

    // Java MessageEventReceivedCmd: FlowableException → 500 (no period).
    Err(ApiError::InternalServerError(format!(
        "Execution '{}' does not have a subscription to a message event with name '{}'",
        execution_id, message_name
    )))
}

/// Java `ExecutionCollectionResource.executeExecutionAction`
/// (ExecutionCollectionResource.java:137-157): only `signalEventReceived` is
/// legal; broadcasts the signal to all subscribed executions and always
/// answers 204.
pub(crate) async fn execute_execution_collection_action(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: ExecutionActionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let action = request.action.as_deref().unwrap_or("null");
    if action != "signalEventReceived" {
        return Err(ApiError::bad_request(format!(
            "Illegal action: '{}'.",
            action
        )));
    }

    // Java checks null only, and the collection variant uses a trailing period.
    let signal_name = request
        .signal_name
        .clone()
        .ok_or_else(|| ApiError::bad_request("Signal name is required."))?;

    let variables = parse_trigger_variables(request.variables)?;
    super::signals::trigger_signal(engine, &signal_name, &variables, None)?;
    Ok(StatusCode::NO_CONTENT)
}
