use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use crate::query_variable::{
    QueryVariableOperation, validate_name_less_equals, validate_operation_value, value_matches,
};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{Path, Query as AxumQuery, Request},
    http::{HeaderMap, StatusCode, Uri, header},
    response::Response,
    routing::{get, post},
};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::task_service::TaskUpdate;
use flowable_engine::engine::variable_service::VariableInstance;
use flowable_engine::history::historic_entities::HistoricTaskInstance;
use flowable_engine::task::Task;
use flowable_form_service::FlowableFormService;
use flowable_task_service::FlowableTaskService;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

const TASKS_PATH: &str = "/runtime/tasks";
const TASK_PATH: &str = "/runtime/tasks/:id";
const TASK_SUBTASKS_PATH: &str = "/runtime/tasks/:id/subtasks";
const TASKS_QUERY_PATH: &str = "/query/tasks";
const TASK_FORM_PATH: &str = "/runtime/tasks/:id/form";
const TASK_COMPLETE_PATH: &str = "/runtime/tasks/:id/complete";
const TASK_ATTACHMENTS_PATH: &str = "/runtime/tasks/:id/attachments";
const TASK_ATTACHMENT_PATH: &str = "/runtime/tasks/:id/attachments/:attachment_id";
const TASK_ATTACHMENT_CONTENT_PATH: &str = "/runtime/tasks/:id/attachments/:attachment_id/content";
const TASK_COMMENTS_PATH: &str = "/runtime/tasks/:id/comments";
const TASK_COMMENT_PATH: &str = "/runtime/tasks/:id/comments/:comment_id";
const TASK_EVENTS_PATH: &str = "/runtime/tasks/:id/events";
const TASK_EVENT_PATH: &str = "/runtime/tasks/:id/events/:event_id";

pub fn router(content_service: super::content::DynContentService) -> Router {
    router_with_prefix("", content_service)
}

fn router_with_prefix(prefix: &str, content_service: super::content::DynContentService) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{TASKS_PATH}"),
            get(list).post(create_task).put(bulk_update_tasks),
        )
        .route(
            &format!("{prefix}{TASK_PATH}"),
            get(get_task)
                .put(update_task)
                .post(action)
                .delete(delete_task),
        )
        .route(
            &format!("{prefix}{TASK_SUBTASKS_PATH}"),
            get(list_sub_tasks),
        )
        .route(&format!("{prefix}{TASKS_QUERY_PATH}"), post(query_tasks))
        .route(&format!("{prefix}{TASK_FORM_PATH}"), get(get_form))
        .route(&format!("{prefix}{TASK_COMPLETE_PATH}"), post(complete))
        .route(
            &format!("{prefix}{TASK_ATTACHMENTS_PATH}"),
            get(list_task_attachments).post(create_task_attachment),
        )
        .route(
            &format!("{prefix}{TASK_ATTACHMENT_PATH}"),
            get(get_task_attachment).delete(delete_task_attachment),
        )
        .route(
            &format!("{prefix}{TASK_ATTACHMENT_CONTENT_PATH}"),
            get(get_task_attachment_content),
        )
        .route(
            &format!("{prefix}{TASK_COMMENTS_PATH}"),
            get(list_task_comments).post(create_task_comment),
        )
        .route(
            &format!("{prefix}{TASK_COMMENT_PATH}"),
            get(get_task_comment).delete(delete_task_comment),
        )
        .route(
            &format!("{prefix}{TASK_EVENTS_PATH}"),
            get(list_task_events),
        )
        .route(
            &format!("{prefix}{TASK_EVENT_PATH}"),
            get(get_task_event).delete(delete_task_event),
        )
        .layer(Extension(content_service))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TaskQuery {
    start: usize,
    size: Option<usize>,
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    #[serde(rename = "processInstanceId")]
    pub process_instance_id: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "nameLike")]
    pub name_like: Option<String>,
    #[serde(rename = "taskDefinitionKey")]
    pub task_definition_key: Option<String>,
    #[serde(rename = "taskDefinitionKeyLike")]
    pub task_definition_key_like: Option<String>,
    pub assignee: Option<String>,
    #[serde(rename = "assigneeLike")]
    pub assignee_like: Option<String>,
    pub owner: Option<String>,
    #[serde(rename = "ownerLike")]
    pub owner_like: Option<String>,
    pub unassigned: Option<bool>,
    #[serde(rename = "delegationState")]
    pub delegation_state: Option<String>,
    pub category: Option<String>,
    #[serde(rename = "tenantId")]
    pub tenant_id: Option<String>,
    #[serde(rename = "candidateUser")]
    pub candidate_user: Option<String>,
    #[serde(rename = "candidateGroup")]
    pub candidate_group: Option<String>,
    /// Java GET `candidateGroups` (CSV) → `taskCandidateGroupIn`.
    #[serde(rename = "candidateGroups")]
    pub candidate_groups: Option<StringOrList>,
    /// Java `TaskQueryRequest.candidateGroupIn` (POST /query/tasks).
    #[serde(rename = "candidateGroupIn")]
    pub candidate_group_in: Option<StringOrList>,
    #[serde(rename = "candidateOrAssigned")]
    pub candidate_or_assigned: Option<String>,
    /// Java `ignoreAssignee`: when true, candidate filters keep assigned tasks.
    /// Default (false/absent) matches Java — candidate queries exclude assigned tasks.
    #[serde(rename = "ignoreAssignee")]
    pub ignore_assignee: Option<bool>,
    #[serde(rename = "involvedUser")]
    pub involved_user: Option<String>,
    /// Java engine `taskInvolvedGroups` (not on Java REST TaskQueryRequest; exposed
    /// for embedded/engine parity via REST for observability tests).
    #[serde(rename = "involvedGroups")]
    pub involved_groups: Option<StringOrList>,
    #[serde(rename = "nameLikeIgnoreCase")]
    pub name_like_ignore_case: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "descriptionLike")]
    pub description_like: Option<String>,
    #[serde(rename = "processDefinitionId")]
    pub process_definition_id: Option<String>,
    #[serde(rename = "processDefinitionKey")]
    pub process_definition_key: Option<String>,
    #[serde(rename = "processDefinitionKeyLike")]
    pub process_definition_key_like: Option<String>,
    #[serde(rename = "processDefinitionName")]
    pub process_definition_name: Option<String>,
    #[serde(rename = "processDefinitionNameLike")]
    pub process_definition_name_like: Option<String>,
    #[serde(rename = "processInstanceIdWithChildren")]
    pub process_instance_id_with_children: Option<String>,
    #[serde(rename = "withoutProcessInstanceId")]
    pub without_process_instance_id: Option<bool>,
    #[serde(rename = "processInstanceBusinessKey")]
    pub process_instance_business_key: Option<String>,
    #[serde(rename = "processInstanceBusinessKeyLike")]
    pub process_instance_business_key_like: Option<String>,
    #[serde(rename = "executionId")]
    pub execution_id: Option<String>,
    #[serde(rename = "createdOn")]
    pub created_on: Option<String>,
    #[serde(rename = "createdBefore")]
    pub created_before: Option<String>,
    #[serde(rename = "createdAfter")]
    pub created_after: Option<String>,
    #[serde(rename = "excludeSubTasks")]
    pub exclude_sub_tasks: Option<bool>,
    #[serde(rename = "taskDefinitionKeys")]
    pub task_definition_keys: Option<StringOrList>,
    #[serde(rename = "withoutCategory")]
    pub without_category: Option<bool>,
    #[serde(rename = "categoryIn")]
    pub category_in: Option<StringOrList>,
    #[serde(rename = "categoryNotIn")]
    pub category_not_in: Option<StringOrList>,
    #[serde(rename = "includeTaskLocalVariables")]
    pub include_task_local_variables: Option<bool>,
    #[serde(rename = "includeProcessVariables")]
    pub include_process_variables: Option<bool>,
    /// accept-but-documented: BPMN tasks carry no CMMN scope columns, so
    /// Java semantics degrade to "matches nothing" when a scope filter is set.
    #[serde(rename = "scopeDefinitionId")]
    pub scope_definition_id: Option<String>,
    #[serde(rename = "scopeId")]
    pub scope_id: Option<String>,
    #[serde(rename = "scopeIds")]
    pub scope_ids: Option<StringOrList>,
    #[serde(rename = "withoutScopeId")]
    pub without_scope_id: Option<bool>,
    #[serde(rename = "scopeType")]
    pub scope_type: Option<String>,
    #[serde(rename = "propagatedStageInstanceId")]
    pub propagated_stage_instance_id: Option<String>,
    /// accept-but-documented: no scope hierarchy in the BPMN-only store.
    #[serde(rename = "rootScopeId")]
    pub root_scope_id: Option<String>,
    #[serde(rename = "parentScopeId")]
    pub parent_scope_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    pub tenant_id_like: Option<String>,
    #[serde(rename = "withoutTenantId")]
    pub without_tenant_id: Option<bool>,
    pub priority: Option<i32>,
    #[serde(rename = "minimumPriority")]
    pub minimum_priority: Option<i32>,
    #[serde(rename = "maximumPriority")]
    pub maximum_priority: Option<i32>,
    #[serde(rename = "dueDate")]
    pub due_date: Option<String>,
    #[serde(rename = "dueBefore")]
    pub due_before: Option<String>,
    #[serde(rename = "dueAfter")]
    pub due_after: Option<String>,
    #[serde(rename = "withoutDueDate")]
    pub without_due_date: Option<bool>,
    /// Java parity: `?suspended=true` filters to suspended tasks only.
    pub suspended: Option<bool>,
    /// Java parity: `?active=true` filters to active (non-suspended) tasks only.
    pub active: Option<bool>,
    #[serde(rename = "taskVariables")]
    pub(crate) task_variables: Option<Vec<QueryVariable>>,
    #[serde(rename = "processInstanceVariables")]
    pub(crate) process_instance_variables: Option<Vec<QueryVariable>>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

/// Java GET parameters pass multi-value filters as CSV strings while
/// POST /query/tasks passes JSON arrays; both deserialize into this enum.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StringOrList {
    Csv(String),
    List(Vec<String>),
}

impl StringOrList {
    fn values(&self) -> Vec<String> {
        match self {
            StringOrList::Csv(csv) => csv.split(',').map(str::to_string).collect(),
            StringOrList::List(values) => values.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryVariable {
    name: Option<String>,
    operation: Option<String>,
    value: Option<serde_json::Value>,
    /// Java `QueryVariable.type` (QueryVariable.java:66-71): accepted for JSON
    /// parity but not used for value conversion — matching is driven by the
    /// JSON value shape (P108 deviation, see query_variable.rs:21-27).
    #[serde(rename = "type")]
    _variable_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateTaskCommentRequest {
    /// Java REST only rejects null/missing (`TaskCommentCollectionResource`);
    /// empty string and whitespace-only messages are accepted.
    message: Option<String>,
    save_process_instance_id: Option<bool>,
    /// Present on Java `CommentRequest` but ignored by
    /// `TaskCommentCollectionResource.createComment` (always TYPE_COMMENT).
    /// Accepted so clients that send `type` are not rejected; typed creation
    /// goes through the engine service API.
    #[serde(default, rename = "type")]
    #[allow(dead_code)]
    comment_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskActionRequest {
    action: String,
    assignee: Option<String>,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    /// Java `TaskActionRequest.formDefinitionId` — when set, complete uses
    /// `CompleteTaskWithFormCmd` (form instance + outcome + variables).
    #[serde(rename = "formDefinitionId")]
    form_definition_id: Option<String>,
    /// Java `TaskActionRequest.outcome` — persisted on form instance.
    outcome: Option<String>,
    variables: Option<Vec<TaskActionVariableRequest>>,
    transient_variables: Option<Vec<TaskActionVariableRequest>>,
    local_scope: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskActionVariableRequest {
    #[serde(flatten)]
    request: super::process_instances::VariableRequest,
    scope: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct UpdateTaskRequest {
    name: Option<Option<String>>,
    description: Option<Option<String>>,
    assignee: Option<Option<String>>,
    owner: Option<Option<String>>,
    delegation_state: Option<Option<String>>,
    parent_task_id: Option<Option<String>>,
    priority: Option<Option<i32>>,
    due_date: Option<Option<String>>,
    category: Option<Option<String>>,
    form_key: Option<Option<String>>,
    tenant_id: Option<Option<String>>,
}

impl UpdateTaskRequest {
    fn into_update(self) -> Result<TaskUpdate, ApiError> {
        let name = match self.name {
            Some(Some(name)) => Some(name),
            Some(None) => {
                return Err(ApiError::bad_request(
                    "Task name cannot be null for update".to_string(),
                ));
            }
            None => None,
        };

        Ok(TaskUpdate {
            name,
            description: self.description,
            assignee: self.assignee,
            owner: self.owner,
            delegation_state: self.delegation_state,
            parent_task_id: self.parent_task_id,
            priority: self.priority,
            due_date: self
                .due_date
                .map(parse_optional_task_due_date)
                .transpose()?,
            category: self.category,
            form_key: self.form_key,
            tenant_id: self.tenant_id,
        })
    }
}

fn parse_update_task_request(body: &str) -> Result<UpdateTaskRequest, ApiError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::bad_request("Task update body must be a JSON object"))?;
    let supported_fields = [
        "name",
        "description",
        "assignee",
        "owner",
        "delegationState",
        "parentTaskId",
        "priority",
        "dueDate",
        "category",
        "formKey",
        "tenantId",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if let Some(unknown_field) = object
        .keys()
        .map(String::as_str)
        .find(|field| !supported_fields.contains(field))
    {
        return Err(ApiError::bad_request(format!(
            "Unsupported task update field '{unknown_field}'"
        )));
    }

    Ok(UpdateTaskRequest {
        name: optional_nullable_string_field(object, "name")?,
        description: optional_nullable_string_field(object, "description")?,
        assignee: optional_nullable_string_field(object, "assignee")?,
        owner: optional_nullable_string_field(object, "owner")?,
        delegation_state: optional_nullable_string_field(object, "delegationState")?,
        parent_task_id: optional_nullable_string_field(object, "parentTaskId")?,
        priority: optional_nullable_i32_field(object, "priority")?,
        due_date: optional_nullable_due_date_field(object, "dueDate")?,
        category: optional_nullable_string_field(object, "category")?,
        form_key: optional_nullable_string_field(object, "formKey")?,
        tenant_id: optional_nullable_string_field(object, "tenantId")?,
    })
}

fn optional_nullable_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<Option<String>>, ApiError> {
    match object.get(field) {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(value)) => Ok(Some(Some(value.clone()))),
        Some(_) => Err(ApiError::bad_request(format!(
            "Task update field '{field}' must be a string or null"
        ))),
    }
}

fn optional_nullable_i32_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<Option<i32>>, ApiError> {
    match object.get(field) {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::Number(value)) => {
            let value = value.as_i64().and_then(|value| i32::try_from(value).ok());
            value.map(|value| Some(Some(value))).ok_or_else(|| {
                ApiError::bad_request(format!(
                    "Task update field '{field}' must be a 32-bit integer"
                ))
            })
        }
        Some(_) => Err(ApiError::bad_request(format!(
            "Task update field '{field}' must be a number or null"
        ))),
    }
}

fn optional_nullable_due_date_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<Option<String>>, ApiError> {
    match object.get(field) {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(value)) => Ok(Some(Some(value.clone()))),
        Some(serde_json::Value::Number(value)) => Ok(Some(Some(value.to_string()))),
        Some(_) => Err(ApiError::bad_request(format!(
            "Task update field '{field}' must be a date string, timestamp, or null"
        ))),
    }
}

struct TaskCompleteVariables {
    variables: Vec<TaskActionVariableRequest>,
    transient_variables: Vec<TaskActionVariableRequest>,
    local_scope: bool,
}

impl TaskQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "processInstanceId")]
    pub process_instance_id: String,
    #[serde(rename = "executionId")]
    pub execution_id: String,
    #[serde(rename = "taskDefinitionKey")]
    pub task_definition_key: String,
    pub assignee: Option<String>,
    pub owner: Option<String>,
    #[serde(rename = "delegationState")]
    pub delegation_state: Option<String>,
    #[serde(rename = "candidateUsers")]
    pub candidate_users: Vec<String>,
    #[serde(rename = "candidateGroups")]
    pub candidate_groups: Vec<String>,
    #[serde(rename = "parentTaskId")]
    pub parent_task_id: Option<String>,
    pub priority: Option<i32>,
    #[serde(rename = "dueDate")]
    pub due_date: Option<String>,
    #[serde(rename = "claimTime")]
    pub claim_time: Option<String>,
    pub category: Option<String>,
    #[serde(rename = "formKey")]
    pub form_key: Option<String>,
    #[serde(rename = "tenantId")]
    pub tenant_id: Option<String>,
    pub state: String,
    /// Java parity: REST returns "active" or "suspended" string, not integer.
    #[serde(rename = "suspensionState")]
    pub suspension_state: String,
    /// Java `TaskResponse.variables`: populated by the
    /// `includeTaskLocalVariables`/`includeProcessVariables` query flags.
    pub variables: Vec<serde_json::Value>,
}

/// Java `AttachmentResponse` — shared with process-instance attachment extension.
pub(crate) use super::attachments::AttachmentResponse as TaskAttachmentResponse;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskCommentResponse {
    pub id: String,
    pub task_url: Option<String>,
    pub process_instance_url: Option<String>,
    pub message: String,
    pub author: Option<String>,
    pub time: String,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    /// Comment type (`comment`, `event`, or custom). Java engine model exposes
    /// `Comment.getType()`; included here so REST consumers can observe typed
    /// comments created through the engine service API.
    #[serde(rename = "type")]
    pub comment_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskEventResponse {
    pub action: String,
    pub id: String,
    pub message: Vec<String>,
    pub task_url: String,
    pub time: String,
    pub url: String,
    pub user_id: Option<String>,
}

fn to_task_response(engine: &ProcessEngine, task: Task) -> TaskResponse {
    let (candidate_users, candidate_groups) = candidate_identity_ids(engine, &task.id);
    // Java parity: REST returns "active" or "suspended" string
    let suspension_state = if task.is_suspended() {
        "suspended".to_string()
    } else {
        "active".to_string()
    };
    TaskResponse {
        id: task.id,
        name: task.name,
        description: task.description,
        process_instance_id: task.process_instance_id,
        execution_id: task.execution_id,
        task_definition_key: task.task_definition_key,
        assignee: task.assignee,
        owner: task.owner,
        delegation_state: task.delegation_state,
        candidate_users,
        candidate_groups,
        parent_task_id: task.parent_task_id,
        priority: task.priority,
        due_date: task.due_date.map(|due_date| due_date.to_rfc3339()),
        claim_time: task.claim_time.map(|claim_time| claim_time.to_rfc3339()),
        category: task.category,
        form_key: task.form_key,
        tenant_id: task.tenant_id,
        state: task.state,
        suspension_state,
        variables: Vec::new(),
    }
}

pub(crate) fn candidate_identity_ids(
    engine: &ProcessEngine,
    task_id: &str,
) -> (Vec<String>, Vec<String>) {
    let mut users = Vec::new();
    let mut groups = Vec::new();
    let mut session = engine.get_runtime_store().create_session().unwrap();
    for link in engine
        .get_runtime_store()
        .find_identity_links_by_task(task_id, &mut session)
    {
        if link.link_type != "candidate" {
            continue;
        }
        if let Some(user_id) = link.user_id
            && !users.contains(&user_id)
        {
            users.push(user_id);
        }
        if let Some(group_id) = link.group_id
            && !groups.contains(&group_id)
        {
            groups.push(group_id);
        }
    }
    (users, groups)
}

pub(crate) async fn list(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<TaskResponse>>, ApiError> {
    let query: TaskQuery = parse_query(&uri)?;
    Ok(Json(tasks_for_query(engine, query)?))
}

pub(crate) async fn query_tasks(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<TaskResponse>>, ApiError> {
    let mut query: TaskQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: TaskQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);
    Ok(Json(tasks_for_query(engine, query)?))
}

fn tasks_for_query(
    engine: Arc<ProcessEngine>,
    query: TaskQuery,
) -> Result<PagedResponse<TaskResponse>, ApiError> {
    let paging = query.paging();
    let task_service = FlowableTaskService::new(Arc::clone(&engine));
    let mut task_query = task_service.create_task_query();

    if let Some(process_instance_id) = query.process_instance_id.clone() {
        task_query = task_query.process_instance_id(process_instance_id);
    }
    if let Some(name) = query.name.clone() {
        task_query = task_query.task_name(name);
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
    if let Some(category) = query.category.clone() {
        task_query = task_query.task_category(category);
    }
    if let Some(tenant_id) = query.tenant_id.clone() {
        task_query = task_query.task_tenant_id(tenant_id);
    }
    if let Some(candidate_user) = query.candidate_user.clone() {
        task_query = task_query.task_candidate_user(candidate_user);
    }
    if let Some(candidate_group) = query.candidate_group.clone() {
        task_query = task_query.task_candidate_group(candidate_group);
    }
    // Java TaskQuery.ignoreAssigneeValue — only meaningful with candidate filters.
    if query.ignore_assignee == Some(true) {
        task_query = task_query.ignore_assignee_value();
    }
    if let Some(involved_groups) = query.involved_groups.as_ref() {
        let groups = involved_groups.values();
        if !groups.is_empty() {
            task_query = task_query.task_involved_groups(groups);
        }
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
        task_query =
            task_query.task_due_date_millis(parse_task_timestamp_millis("dueDate", due_date)?);
    }
    if let Some(due_before) = query.due_before.as_deref() {
        task_query = task_query
            .task_due_before_millis(parse_task_timestamp_millis("dueBefore", due_before)?);
    }
    if let Some(due_after) = query.due_after.as_deref() {
        task_query =
            task_query.task_due_after_millis(parse_task_timestamp_millis("dueAfter", due_after)?);
    }
    if query.without_due_date == Some(true) {
        task_query = task_query.task_without_due_date();
    }
    // Java parity: ?suspended=true / ?active=true query parameters
    if query.suspended == Some(true) {
        task_query = task_query.suspended();
    }
    if query.active == Some(true) {
        task_query = task_query.active();
    }

    let mut tasks = task_query.list()?;
    if let Some(name_like) = query.name_like.as_deref() {
        tasks.retain(|task| sql_like_matches(name_like, &task.name));
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
    if query.unassigned == Some(true) {
        tasks.retain(|task| task.assignee.is_none());
    }
    if let Some(delegation_state) = query.delegation_state.as_deref() {
        validate_task_delegation_state(delegation_state)?;
        tasks.retain(|task| task.delegation_state.as_deref() == Some(delegation_state));
    }
    if let Some(task_variables) = query.task_variables.as_ref() {
        apply_task_local_variable_filters(&engine, &mut tasks, task_variables)?;
    }
    if let Some(process_instance_variables) = query.process_instance_variables.as_ref() {
        let variable_instances = engine
            .get_variable_service()
            .create_variable_instance_query()
            .list()?;
        apply_task_process_variable_filters(
            &mut tasks,
            &variable_instances,
            process_instance_variables,
        )?;
    }
    if let Some(task_id) = query.task_id.as_deref() {
        tasks.retain(|task| task.id == task_id);
    }
    if let Some(name_like_ignore_case) = query.name_like_ignore_case.as_deref() {
        let pattern = name_like_ignore_case.to_lowercase();
        tasks.retain(|task| sql_like_matches(&pattern, &task.name.to_lowercase()));
    }
    if let Some(description) = query.description.as_deref() {
        tasks.retain(|task| task.description.as_deref() == Some(description));
    }
    if let Some(description_like) = query.description_like.as_deref() {
        tasks.retain(|task| {
            task.description
                .as_deref()
                .is_some_and(|description| sql_like_matches(description_like, description))
        });
    }
    if let Some(execution_id) = query.execution_id.as_deref() {
        tasks.retain(|task| task.execution_id == execution_id);
    }
    if query.exclude_sub_tasks == Some(true) {
        tasks.retain(|task| task.parent_task_id.is_none());
    }
    if query.without_process_instance_id == Some(true) {
        tasks.retain(|task| task.process_instance_id.is_empty());
    }
    if let Some(task_definition_keys) = query.task_definition_keys.as_ref() {
        let keys = task_definition_keys.values();
        tasks.retain(|task| keys.iter().any(|key| *key == task.task_definition_key));
    }
    if query.without_category == Some(true) {
        tasks.retain(|task| task.category.is_none());
    }
    if let Some(category_in) = query.category_in.as_ref() {
        let categories = category_in.values();
        tasks.retain(|task| {
            task.category
                .as_deref()
                .is_some_and(|category| categories.iter().any(|c| c == category))
        });
    }
    if let Some(category_not_in) = query.category_not_in.as_ref() {
        let categories = category_not_in.values();
        // Java SQL `CATEGORY_ NOT IN (...)`: null categories never match.
        tasks.retain(|task| {
            task.category
                .as_deref()
                .is_some_and(|category| !categories.iter().any(|c| c == category))
        });
    }
    if let Some(created_on) = query.created_on.as_deref() {
        let millis = parse_task_timestamp_millis("createdOn", created_on)?;
        tasks.retain(|task| {
            task.created_time
                .is_some_and(|time| time.timestamp_millis() == millis)
        });
    }
    if let Some(created_before) = query.created_before.as_deref() {
        let millis = parse_task_timestamp_millis("createdBefore", created_before)?;
        tasks.retain(|task| {
            task.created_time
                .is_some_and(|time| time.timestamp_millis() < millis)
        });
    }
    if let Some(created_after) = query.created_after.as_deref() {
        let millis = parse_task_timestamp_millis("createdAfter", created_after)?;
        tasks.retain(|task| {
            task.created_time
                .is_some_and(|time| time.timestamp_millis() > millis)
        });
    }
    if let Some(tenant_id_like) = query.tenant_id_like.as_deref() {
        tasks.retain(|task| {
            task.tenant_id
                .as_deref()
                .is_some_and(|tenant_id| sql_like_matches(tenant_id_like, tenant_id))
        });
    }
    if query.without_tenant_id == Some(true) {
        tasks.retain(|task| task.tenant_id.as_deref().unwrap_or("").is_empty());
    }
    let ignore_assignee = query.ignore_assignee == Some(true);
    if let Some(candidate_groups) = query
        .candidate_groups
        .as_ref()
        .or(query.candidate_group_in.as_ref())
    {
        // Java Task.xml candidateGroupIn: ASSIGNEE_ is null unless ignoreAssignee.
        let groups = candidate_groups.values();
        tasks.retain(|task| {
            if !ignore_assignee && task.assignee.is_some() {
                return false;
            }
            let (_, task_groups) = candidate_identity_ids(&engine, &task.id);
            task_groups.iter().any(|group| groups.contains(group))
        });
    }
    if let Some(involved_user) = query.involved_user.as_deref() {
        // Java `taskInvolvedUser`: assignee, owner or any identity link user.
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        tasks.retain(|task| {
            task.assignee.as_deref() == Some(involved_user)
                || task.owner.as_deref() == Some(involved_user)
                || store
                    .find_identity_links_by_task(&task.id, &mut session)
                    .iter()
                    .any(|link| link.user_id.as_deref() == Some(involved_user))
        });
    }
    if let Some(candidate_or_assigned) = query.candidate_or_assigned.as_deref() {
        // Java `taskCandidateOrAssigned`: assignee matches, or (optionally only
        // when unassigned) the user is a candidate (directly or via groups).
        // ignoreAssignee relaxes the unassigned requirement on the candidate arm
        // (Java Task.xml bothCandidateAndAssigned + ignoreAssigneeValue).
        let user_groups: Vec<String> = engine
            .get_identity_service()
            .get_groups_by_user(candidate_or_assigned)
            .into_iter()
            .map(|group| group.id)
            .collect();
        tasks.retain(|task| {
            if task.assignee.as_deref() == Some(candidate_or_assigned) {
                return true;
            }
            if !ignore_assignee && task.assignee.is_some() {
                return false;
            }
            let (candidate_users, candidate_groups) = candidate_identity_ids(&engine, &task.id);
            candidate_users.iter().any(|u| u == candidate_or_assigned)
                || candidate_groups
                    .iter()
                    .any(|group| user_groups.contains(group))
        });
    }
    let needs_process_instance_filter = query.process_definition_id.is_some()
        || query.process_definition_key.is_some()
        || query.process_definition_key_like.is_some()
        || query.process_definition_name.is_some()
        || query.process_definition_name_like.is_some()
        || query.process_instance_business_key.is_some()
        || query.process_instance_business_key_like.is_some();
    if needs_process_instance_filter {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        tasks.retain(|task| {
            let Some(pi) = store.find_process_instance(&task.process_instance_id, &mut session)
            else {
                return false;
            };
            if let Some(v) = query.process_definition_id.as_deref()
                && pi.process_definition_id != v
            {
                return false;
            }
            if let Some(v) = query.process_definition_key.as_deref()
                && pi.process_definition_key != v
            {
                return false;
            }
            if let Some(v) = query.process_definition_key_like.as_deref()
                && !sql_like_matches(v, &pi.process_definition_key)
            {
                return false;
            }
            if let Some(v) = query.process_definition_name.as_deref()
                && pi.process_definition_name.as_deref() != Some(v)
            {
                return false;
            }
            if let Some(v) = query.process_definition_name_like.as_deref()
                && !pi
                    .process_definition_name
                    .as_deref()
                    .is_some_and(|name| sql_like_matches(v, name))
            {
                return false;
            }
            if let Some(v) = query.process_instance_business_key.as_deref()
                && pi.business_key.as_deref() != Some(v)
            {
                return false;
            }
            if let Some(v) = query.process_instance_business_key_like.as_deref()
                && !pi
                    .business_key
                    .as_deref()
                    .is_some_and(|key| sql_like_matches(v, key))
            {
                return false;
            }
            true
        });
    }
    if let Some(target) = query.process_instance_id_with_children.as_deref() {
        // Java `processInstanceIdWithChildren`: the task's process instance
        // or any ancestor along the call-activity chain matches the id.
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        tasks.retain(|task| {
            if task.process_instance_id.is_empty() {
                return false;
            }
            let mut current = task.process_instance_id.clone();
            loop {
                if current == target {
                    return true;
                }
                let Some(pi) = store.find_process_instance(&current, &mut session) else {
                    return false;
                };
                let Some(super_execution_id) = pi.super_execution_id else {
                    return false;
                };
                let Some(execution) = store.find_execution(&super_execution_id, &mut session)
                else {
                    return false;
                };
                let Some(parent_instance_id) = execution.process_instance_id else {
                    return false;
                };
                current = parent_instance_id;
            }
        });
    }
    // accept-but-documented: the BPMN-only store has no CMMN scope columns, so
    // any scope filter matches Java semantics for BPMN tasks (scope columns
    // are always null → no task matches).
    if query.scope_id.is_some()
        || query.scope_definition_id.is_some()
        || query.scope_type.is_some()
        || query.scope_ids.is_some()
        || query.propagated_stage_instance_id.is_some()
    {
        tasks.clear();
    }
    // accept-but-documented: `withoutScopeId=true` matches every BPMN task,
    // and `rootScopeId`/`parentScopeId` are accepted without effect (no scope
    // hierarchy in the BPMN-only store). `ignoreAssignee` is applied above for
    // candidate / candidateOrAssigned filters (Java Task.xml parity).
    let _ = (
        query.without_scope_id,
        query.root_scope_id.as_deref(),
        query.parent_scope_id.as_deref(),
    );
    sort_tasks(&mut tasks, query.sort.as_deref(), query.order.as_deref())?;

    let include_task_local_variables = query.include_task_local_variables == Some(true);
    let include_process_variables = query.include_process_variables == Some(true);
    let process_variable_instances = if include_process_variables {
        engine
            .get_variable_service()
            .create_variable_instance_query()
            .list()?
    } else {
        Vec::new()
    };

    let mut result = Vec::new();
    for task in tasks {
        let mut response = to_task_response(&engine, task);
        if include_task_local_variables {
            let mut locals: Vec<(String, serde_json::Value)> = engine
                .get_task_service()
                .get_task_local_variables(response.id.clone())?
                .into_iter()
                .collect();
            locals.sort_by(|left, right| left.0.cmp(&right.0));
            for (name, value) in locals {
                response
                    .variables
                    .push(task_variable_response(&name, &value, "local"));
            }
        }
        if include_process_variables && !response.process_instance_id.is_empty() {
            for instance in process_variable_instances
                .iter()
                .filter(|instance| instance.process_instance_id == response.process_instance_id)
            {
                response.variables.push(task_variable_response(
                    &instance.name,
                    &instance.value,
                    "global",
                ));
            }
        }
        result.push(response);
    }

    Ok(paging.paginate(result))
}

/// Java `RestVariable` shape used inside `TaskResponse.variables`.
fn task_variable_response(name: &str, value: &serde_json::Value, scope: &str) -> serde_json::Value {
    let response =
        super::process_instances::to_rest_variable_response(name.to_string(), value.clone());
    serde_json::json!({
        "name": response.name,
        "type": response.variable_type,
        "value": response.value,
        "scope": scope,
    })
}

fn apply_task_local_variable_filters(
    engine: &ProcessEngine,
    tasks: &mut Vec<Task>,
    variables: &[QueryVariable],
) -> Result<(), ApiError> {
    for variable in variables {
        let operation = parse_query_variable_operation(variable)?;
        let value = query_variable_value(variable)?;
        let name = variable.name.as_deref();

        validate_query_variable(variable, operation, value)?;

        let mut filtered = Vec::new();
        for task in std::mem::take(tasks) {
            let task_variables = engine
                .get_task_service()
                .get_task_local_variables(task.id.clone())?;
            if task_variables
                .iter()
                .any(|(candidate_name, candidate_value)| {
                    variable_value_matches(candidate_name, candidate_value, name, operation, value)
                })
            {
                filtered.push(task);
            }
        }
        *tasks = filtered;
    }

    Ok(())
}

fn apply_task_process_variable_filters(
    tasks: &mut Vec<Task>,
    variable_instances: &[VariableInstance],
    variables: &[QueryVariable],
) -> Result<(), ApiError> {
    for variable in variables {
        let operation = parse_query_variable_operation(variable)?;
        let value = query_variable_value(variable)?;
        let name = variable.name.as_deref();

        validate_query_variable(variable, operation, value)?;

        tasks.retain(|task| {
            variable_instances
                .iter()
                .filter(|candidate| candidate.process_instance_id == task.process_instance_id)
                .any(|candidate| {
                    variable_value_matches(
                        &candidate.name,
                        &candidate.value,
                        name,
                        operation,
                        value,
                    )
                })
        });
    }

    Ok(())
}

fn parse_query_variable_operation(
    variable: &QueryVariable,
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

fn query_variable_value(variable: &QueryVariable) -> Result<&serde_json::Value, ApiError> {
    variable.value.as_ref().ok_or_else(|| {
        ApiError::bad_request(format!(
            "Variable value is missing for variable: {}",
            variable.name.as_deref().unwrap_or("null")
        ))
    })
}

fn validate_query_variable(
    variable: &QueryVariable,
    operation: QueryVariableOperation,
    value: &serde_json::Value,
) -> Result<(), ApiError> {
    validate_name_less_equals(variable.name.as_deref(), operation)?;
    validate_operation_value(operation, value)
}

fn variable_value_matches(
    candidate_name: &str,
    candidate_value: &serde_json::Value,
    expected_name: Option<&str>,
    operation: QueryVariableOperation,
    expected_value: &serde_json::Value,
) -> bool {
    if expected_name.is_some_and(|name| candidate_name != name) {
        return false;
    }

    value_matches(candidate_value, operation, expected_value)
}

fn sort_tasks(tasks: &mut [Task], sort: Option<&str>, order: Option<&str>) -> Result<(), ApiError> {
    match sort {
        None => {}
        Some("id") => tasks.sort_by(|left, right| left.id.cmp(&right.id)),
        Some("name") => {
            tasks.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)))
        }
        Some("description") => tasks.sort_by(|left, right| {
            left.description
                .cmp(&right.description)
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
        Some("created") | Some("createTime") => tasks.sort_by(|left, right| {
            left.created_time
                .cmp(&right.created_time)
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
            left.category
                .cmp(&right.category)
                .then(left.id.cmp(&right.id))
        }),
        Some("tenantId") => tasks.sort_by(|left, right| {
            left.tenant_id
                .cmp(&right.tenant_id)
                .then(left.id.cmp(&right.id))
        }),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported task sort field '{other}'"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => tasks.reverse(),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported task sort order '{other}'"
            )));
        }
    }

    Ok(())
}

fn validate_task_delegation_state(delegation_state: &str) -> Result<(), ApiError> {
    match delegation_state {
        "pending" | "resolved" => Ok(()),
        other => Err(ApiError::bad_request(format!(
            "Unsupported task delegationState '{other}'"
        ))),
    }
}

/// Max Unicode scalar count for in-memory SQL-LIKE filter operands.
///
/// Bound is on **characters** (same unit as the matcher). Re-export of the
/// shared P143 constant for local tests and any in-crate callers.
pub(crate) const MAX_SQL_LIKE_LEN: usize = flowable_engine_common::like::MAX_SQL_LIKE_LEN;

/// SQL-LIKE style match for in-memory filters (`%` any sequence, `_` one char,
/// other chars literal). Case-sensitive; callers lower-case both sides for
/// ignore-case variants.
///
/// Space is O(value length) via two rolling rows (not O(n×m) full DP matrix /
/// deep recursion). Used by tasks, models, deployments, and other callers.
/// Thin wrapper over the P143 unified implementation in `flowable_engine_common`.
pub(crate) fn sql_like_matches(pattern: &str, value: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}

fn parse_task_timestamp_millis(field_name: &str, value: &str) -> Result<i64, ApiError> {
    if let Ok(timestamp) = value.parse::<i64>() {
        return Ok(timestamp);
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .map_err(|error| {
            ApiError::BadRequest(format!("Invalid {field_name} '{}': {}", value, error))
        })
}

fn parse_task_due_date(field_name: &str, value: &str) -> Result<DateTime<Utc>, ApiError> {
    if let Ok(timestamp) = value.parse::<i64>() {
        return DateTime::<Utc>::from_timestamp_millis(timestamp).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Invalid {field_name} '{}': timestamp out of range",
                value
            ))
        });
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| {
            ApiError::BadRequest(format!("Invalid {field_name} '{}': {}", value, error))
        })
}

fn parse_optional_task_due_date(value: Option<String>) -> Result<Option<DateTime<Utc>>, ApiError> {
    value
        .as_deref()
        .map(|value| parse_task_due_date("dueDate", value))
        .transpose()
}

pub(crate) async fn get_task(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
) -> Result<Json<TaskResponse>, ApiError> {
    let task = load_task(&engine, &id)?;

    Ok(Json(to_task_response(&engine, task)))
}

/// Java `TaskCollectionResource.createTask`: POST /runtime/tasks → 201 with
/// the created (standalone) task.
pub(crate) async fn create_task(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<(StatusCode, Json<TaskResponse>), ApiError> {
    let request = parse_update_task_request(&body)?;
    let mut task = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    apply_task_request_fields(request, &mut task)?;
    let task = engine.get_task_service().create_task(task)?;
    Ok((StatusCode::CREATED, Json(to_task_response(&engine, task))))
}

/// Java `TaskRequest` setter semantics: only fields present in the JSON body
/// are applied (`xxxSet` markers).
fn apply_task_request_fields(request: UpdateTaskRequest, task: &mut Task) -> Result<(), ApiError> {
    if let Some(name) = request.name {
        task.name = name.unwrap_or_default();
    }
    if let Some(description) = request.description {
        task.description = description;
    }
    if let Some(assignee) = request.assignee {
        task.assignee = assignee;
    }
    if let Some(owner) = request.owner {
        task.owner = owner;
    }
    if let Some(delegation_state) = request.delegation_state {
        // Java `TaskRequest.getDelegationState` message, verbatim.
        if let Some(state) = delegation_state.as_deref()
            && state != "pending"
            && state != "resolved"
        {
            return Err(ApiError::bad_request(format!(
                "Illegal value for delegationState: {state}"
            )));
        }
        task.delegation_state = delegation_state;
    }
    if let Some(parent_task_id) = request.parent_task_id {
        task.parent_task_id = parent_task_id;
    }
    if let Some(priority) = request.priority {
        task.priority = priority;
    }
    if let Some(due_date) = request.due_date {
        task.due_date = parse_optional_task_due_date(due_date)?;
    }
    if let Some(category) = request.category {
        task.category = category;
    }
    if let Some(form_key) = request.form_key {
        task.form_key = form_key;
    }
    if let Some(tenant_id) = request.tenant_id {
        task.tenant_id = tenant_id;
    }
    Ok(())
}

/// Java `TaskCollectionResource.bulkUpdateTasks`: PUT /runtime/tasks.
pub(crate) async fn bulk_update_tasks(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<Json<PagedResponse<TaskResponse>>, ApiError> {
    if body.trim().is_empty() || body.trim() == "null" {
        // Java: a missing body raises a plain FlowableException (500).
        return Err(ApiError::InternalServerError(
            "A request body was expected when bulk updating tasks.".to_string(),
        ));
    }
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::bad_request("Task update body must be a JSON object"))?;

    let task_ids = match object.get("taskIds") {
        None | Some(serde_json::Value::Null) => {
            return Err(ApiError::bad_request(
                "taskIds can not be null for bulk update tasks requests".to_string(),
            ));
        }
        Some(serde_json::Value::Array(ids)) => ids
            .iter()
            .map(|id| {
                id.as_str().map(str::to_string).ok_or_else(|| {
                    ApiError::bad_request("taskIds must be an array of strings".to_string())
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(ApiError::bad_request(
                "taskIds must be an array of strings".to_string(),
            ));
        }
    };

    let mut update_object = object.clone();
    update_object.remove("taskIds");
    let request = parse_update_task_request(&serde_json::Value::Object(update_object).to_string())?;
    let update = request.into_update()?;

    // Java `getTasksFromIdList` + size check: 404 lists the missing ids.
    let missing: Vec<String> = task_ids
        .iter()
        .filter(|task_id| load_task(&engine, task_id).is_err())
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(ApiError::NotFound(format!(
            "Could not find task instance with id:{}",
            missing.join(",")
        )));
    }

    let task_service = FlowableTaskService::new(Arc::clone(&engine));
    let mut data = Vec::new();
    for task_id in &task_ids {
        let task = task_service.update_task_by_id(task_id.clone(), update.clone())?;
        data.push(to_task_response(&engine, task));
    }

    // Java returns a bare `DataResponse` with only `data` populated.
    Ok(Json(PagedResponse {
        start: 0,
        size: 0,
        total: 0,
        sort: None,
        order: None,
        data,
    }))
}

pub(crate) async fn update_task(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    body: String,
) -> Result<Json<TaskResponse>, ApiError> {
    let task_service = FlowableTaskService::new(Arc::clone(&engine));
    let request = parse_update_task_request(&body)?;
    let task = task_service.update_task_by_id(id, request.into_update()?)?;

    Ok(Json(to_task_response(&engine, task)))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteTaskQuery {
    cascade_history: Option<bool>,
    delete_reason: Option<String>,
}

/// Java parity: `TaskResource.deleteTask` — 403 for workflow/CMMN tasks,
/// 204 on success. `cascadeHistory` removes the historic task instance too.
pub(crate) async fn delete_task(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    AxumQuery(query): AxumQuery<DeleteTaskQuery>,
) -> Result<StatusCode, ApiError> {
    let task = load_task(&engine, &id)?;

    // Java parity: "Cannot delete a task that is part of a process instance."
    if !task.execution_id.is_empty() {
        return Err(ApiError::Forbidden(
            "Cannot delete a task that is part of a process instance.".to_string(),
        ));
    }

    let task_service = FlowableTaskService::new(Arc::clone(&engine));
    task_service.delete_task(
        id,
        query.delete_reason,
        query.cascade_history.unwrap_or(false),
    )?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_sub_tasks(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TaskResponse>>, ApiError> {
    let task = load_task(&engine, &id)?;
    let task_service = FlowableTaskService::new(Arc::clone(&engine));
    let subtasks = task_service
        .get_sub_tasks(task.id)?
        .into_iter()
        .map(|task| to_task_response(&engine, task))
        .collect();

    Ok(Json(subtasks))
}

pub(crate) async fn complete(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request = parse_canonical_task_complete_request(&body)?;
    let variables = TaskCompleteVariables {
        variables: request.variables.unwrap_or_default(),
        transient_variables: request.transient_variables.unwrap_or_default(),
        local_scope: request.local_scope.unwrap_or(false),
    };
    // Java TaskCompletionBuilderImpl: formDefinitionId != null → CompleteTaskWithFormCmd
    complete_task_action(
        &engine,
        id,
        request.form_definition_id,
        request.outcome,
        variables,
    )?;

    Ok(StatusCode::OK)
}

fn parse_canonical_task_complete_request(body: &str) -> Result<TaskActionRequest, ApiError> {
    if body.trim().is_empty() {
        return Err(ApiError::bad_request(
            "Task complete request must use the canonical 'action: \"complete\"' shape; received empty body",
        ));
    }
    let request: TaskActionRequest = serde_json::from_str(body).map_err(|error| {
        ApiError::bad_request(format!(
            "Task complete request must use the canonical 'action: \"complete\"' shape; {error}"
        ))
    })?;
    if request.action != "complete" {
        return Err(ApiError::bad_request(format!(
            "Task complete request must use the canonical 'action: \"complete\"' shape; received action '{}'",
            request.action
        )));
    }
    Ok(request)
}

pub(crate) async fn action(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    Json(request): Json<TaskActionRequest>,
) -> Result<StatusCode, ApiError> {
    let task_service = FlowableTaskService::new(Arc::clone(&engine));
    match request.action.as_str() {
        "claim" => {
            if request.assignee.is_none() {
                task_service.unclaim_task_by_id(id)?;
                return Ok(StatusCode::OK);
            }
            let assignee = request
                .assignee
                .filter(|assignee| !assignee.trim().is_empty())
                .ok_or_else(|| ApiError::bad_request("Assignee is required for claim action"))?;
            task_service.claim_task_by_id(id, assignee)?;
            Ok(StatusCode::OK)
        }
        "unclaim" => {
            task_service.unclaim_task_by_id(id)?;
            Ok(StatusCode::OK)
        }
        "complete" => {
            let variables = TaskCompleteVariables {
                variables: request.variables.unwrap_or_default(),
                transient_variables: request.transient_variables.unwrap_or_default(),
                local_scope: request.local_scope.unwrap_or(false),
            };
            // Java TaskResource.completeTask → formDefinitionId / outcome on builder
            complete_task_action(
                &engine,
                id,
                request.form_definition_id,
                request.outcome,
                variables,
            )?;
            Ok(StatusCode::OK)
        }
        "delegate" => {
            let user_id =
                required_task_action_user_id(request.assignee, request.user_id, "delegate")?;
            task_service.delegate_task_by_id(id, user_id)?;
            Ok(StatusCode::OK)
        }
        "resolve" => {
            task_service.resolve_task_by_id(id)?;
            Ok(StatusCode::OK)
        }
        other => Err(ApiError::bad_request(format!(
            "Unsupported task action '{other}'"
        ))),
    }
}

fn required_task_action_user_id(
    assignee: Option<String>,
    user_id: Option<String>,
    action: &str,
) -> Result<String, ApiError> {
    assignee
        .filter(|value| !value.trim().is_empty())
        .or_else(|| user_id.filter(|value| !value.trim().is_empty()))
        .ok_or_else(|| ApiError::bad_request(format!("Assignee is required for {action} action")))
}

/// Dispatch complete: with `formDefinitionId` use form-service single-command path;
/// otherwise keep the existing variables-only complete path unchanged.
fn complete_task_action(
    engine: &Arc<ProcessEngine>,
    task_id: String,
    form_definition_id: Option<String>,
    outcome: Option<String>,
    complete_variables: TaskCompleteVariables,
) -> Result<(), ApiError> {
    let local_scope = complete_variables.local_scope;
    let (variables, local_variables) = scoped_variable_requests_to_maps(
        complete_variables.variables,
        complete_variables.local_scope,
    )?;
    let (transient_variables, transient_local_variables) = scoped_variable_requests_to_maps(
        complete_variables.transient_variables,
        complete_variables.local_scope,
    )?;
    if !transient_local_variables.is_empty() {
        return Err(ApiError::bad_request(
            "Task complete does not support local transientVariables".to_string(),
        ));
    }

    if let Some(form_definition_id) = form_definition_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    {
        // Java: TaskCompletionBuilder.formDefinitionId(...).outcome(...).complete()
        // Merge local-scoped variables into the form variable map when localScope
        // is set (CompleteTaskWithFormCmd local vs global split).
        let mut form_variables = variables;
        if local_scope {
            form_variables.extend(local_variables);
        } else {
            // Per-variable local scope from RestVariable.scope=local
            for (name, value) in local_variables {
                form_variables.insert(name, value);
            }
        }
        // When local_scope is true, all variables go to variablesLocal in Java.
        let form_service = FlowableFormService::new(Arc::clone(engine));
        form_service.complete_task_with_form_definition(
            task_id,
            form_definition_id,
            outcome,
            form_variables,
            local_scope,
            transient_variables,
            None,
        )?;
        return Ok(());
    }

    complete_task_with_variables(
        &FlowableTaskService::new(Arc::clone(engine)),
        task_id,
        variables,
        local_variables,
        transient_variables,
    )
}

fn complete_task_with_variables(
    task_service: &FlowableTaskService,
    task_id: String,
    variables: HashMap<String, serde_json::Value>,
    local_variables: HashMap<String, serde_json::Value>,
    transient_variables: HashMap<String, serde_json::Value>,
) -> Result<(), ApiError> {
    if variables.is_empty() && transient_variables.is_empty() {
        if local_variables.is_empty() {
            task_service.complete_task_by_id(task_id)?;
        } else {
            task_service.complete_task_by_id_with_local_variables(task_id, local_variables)?;
        }
    } else {
        for (name, value) in local_variables {
            task_service.set_task_local_variable(task_id.clone(), name, value)?;
        }
        if transient_variables.is_empty() {
            task_service.complete_task_by_id_with_variables(task_id, variables)?;
        } else {
            task_service.complete_task_by_id_with_variable_maps(
                task_id,
                variables,
                transient_variables,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::type_complexity)]
fn scoped_variable_requests_to_maps(
    requests: Vec<TaskActionVariableRequest>,
    default_local_scope: bool,
) -> Result<
    (
        HashMap<String, serde_json::Value>,
        HashMap<String, serde_json::Value>,
    ),
    ApiError,
> {
    let mut global_requests = Vec::new();
    let mut local_requests = Vec::new();
    for request in requests {
        if task_action_variable_is_local(request.scope.as_deref(), default_local_scope)? {
            local_requests.push(request.request);
        } else {
            global_requests.push(request.request);
        }
    }
    Ok((
        super::process_instances::variable_requests_to_map(global_requests)?,
        super::process_instances::variable_requests_to_map(local_requests)?,
    ))
}

fn task_action_variable_is_local(
    scope: Option<&str>,
    default_local_scope: bool,
) -> Result<bool, ApiError> {
    match scope {
        None => Ok(default_local_scope),
        Some(scope) if scope.eq_ignore_ascii_case("local") => Ok(true),
        Some(scope) if scope.eq_ignore_ascii_case("global") => Ok(false),
        Some(scope) => Err(ApiError::bad_request(format!(
            "Invalid variable scope: '{scope}'"
        ))),
    }
}

pub(crate) async fn get_form(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let service = FlowableFormService::new(engine);
    let form_data = service.get_task_form_data(&id)?;
    let definition = service.get_form_definition(&form_data.form_definition_id)?;
    Ok(Json(definition.form_payload))
}

/// Java `TaskAttachmentCollectionResource.createAttachment`: multipart file or
/// JSON `AttachmentRequest`. Runtime task required (create is a write).
pub(crate) async fn create_task_attachment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path(id): Path<String>,
    headers: HeaderMap,
    request: Request,
) -> Result<(StatusCode, Json<TaskAttachmentResponse>), ApiError> {
    // Java: getTaskFromRequestWithoutAccessCheck — runtime task only.
    let task = load_task(&engine, &id)?;

    // Java parity: CreateAttachmentCmd.verifyTaskParameters checks task.isSuspended()
    if task.is_suspended() {
        return Err(ApiError::InternalServerError(format!(
            "It is not allowed to add an attachment to a suspended task '{}'",
            task.id
        )));
    }

    // Java parity: CreateAttachmentCmd.verifyExecutionParameters checks execution.isSuspended()
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    if let Some(pi) = store.find_process_instance(&task.process_instance_id, &mut session) {
        if pi.is_suspended {
            return Err(ApiError::InternalServerError(format!(
                "It is not allowed to add an attachment to a suspended process instance '{}'",
                task.process_instance_id
            )));
        }
    }

    let user_id = user_id_from_basic_auth(&headers);

    let input = if super::attachments::is_multipart_request(&headers) {
        super::attachments::parse_multipart_attachment(request).await?
    } else {
        super::attachments::parse_json_attachment(request).await?
    };

    let item = content_service.create_task_attachment(
        task.id.clone(),
        input.name,
        input.description,
        input.attachment_type,
        input.external_url,
        input.content,
        user_id,
        Some(task.process_instance_id.clone()),
    )?;

    Ok((
        StatusCode::CREATED,
        Json(super::attachments::task_attachment_response_from_record(
            &task.id, item,
        )),
    ))
}

/// Java list: `getHistoricTaskFromRequest` — readable after task completion.
pub(crate) async fn list_task_attachments(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TaskAttachmentResponse>>, ApiError> {
    let task = load_historic_task(&engine, &id)?;
    let attachments = content_service
        .list_task_attachments(&task.id)?
        .into_iter()
        .map(|item| {
            super::attachments::task_attachment_response_from_record(&task.id, item)
        })
        .collect();
    Ok(Json(attachments))
}

/// Java get: historic task + attachment scoped to task.
pub(crate) async fn get_task_attachment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path((id, attachment_id)): Path<(String, String)>,
) -> Result<Json<TaskAttachmentResponse>, ApiError> {
    let task = load_historic_task(&engine, &id)?;
    let item = content_service.get_task_attachment(&task.id, &attachment_id)?;
    Ok(Json(
        super::attachments::task_attachment_response_from_record(&task.id, item),
    ))
}

/// Java `TaskAttachmentContentResource`: historic task; Content-Type from type
/// when it is a valid media type, else `application/octet-stream`.
pub(crate) async fn get_task_attachment_content(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path((id, attachment_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let task = load_historic_task(&engine, &id)?;
    let content = content_service.get_task_attachment_content(&task.id, &attachment_id)?;
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

/// Java delete: runtime task required; 204 empty body.
pub(crate) async fn delete_task_attachment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(content_service): Extension<super::content::DynContentService>,
    Path((id, attachment_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let task = load_task(&engine, &id)?;
    let user_id = user_id_from_basic_auth(&headers);
    content_service.delete_task_attachment(&task.id, &attachment_id, user_id.as_deref())?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn create_task_comment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateTaskCommentRequest>,
) -> Result<(StatusCode, Json<TaskCommentResponse>), ApiError> {
    // Create requires a runtime task (Java `getTaskFromRequestWithoutAccessCheck`).
    let task = load_task(&engine, &id)?;
    // Java `TaskCommentCollectionResource.createComment`: only null message is
    // rejected; empty string and pure whitespace are accepted.
    let message = request
        .message
        .ok_or_else(|| ApiError::bad_request("Comment text is required."))?;
    let process_instance_id = request
        .save_process_instance_id
        .unwrap_or(false)
        .then_some(task.process_instance_id.as_str());
    // Java `AddCommentCmd` sets author from `Authentication.getAuthenticatedUserId()`.
    let author = user_id_from_basic_auth(&headers);
    let comment = engine.get_history_service().create_task_comment(
        &task.id,
        process_instance_id,
        &message,
        author.as_deref(),
    )?;
    Ok((StatusCode::CREATED, Json(task_comment_response(comment))))
}

pub(crate) async fn list_task_comments(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TaskCommentResponse>>, ApiError> {
    // List uses historic task so comments remain readable after completion
    // (Java `TaskCommentCollectionResource.getComments` → getHistoricTaskFromRequest).
    let task = load_historic_task(&engine, &id)?;
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let comments = engine
        .get_history_service()
        .get_task_comments(&task.id, &mut session)
        .into_iter()
        .map(task_comment_response)
        .collect();
    Ok(Json(comments))
}

pub(crate) async fn get_task_comment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, comment_id)): Path<(String, String)>,
) -> Result<Json<TaskCommentResponse>, ApiError> {
    // Get uses historic task (Java `TaskCommentResource.getComment`).
    let task = load_historic_task(&engine, &id)?;
    let comment = task_comment_for_historic(&engine, &task, &comment_id)?;
    Ok(Json(task_comment_response(comment)))
}

pub(crate) async fn delete_task_comment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, comment_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    // Delete requires a runtime task (Java `TaskCommentResource.deleteComment`).
    let task = load_task(&engine, &id)?;
    engine
        .get_history_service()
        .delete_task_comment(&task.id, &comment_id, None)?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Task '{}' comment '{}' was not found",
                task.id, comment_id
            ))
        })?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_task_events(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TaskEventResponse>>, ApiError> {
    // List uses historic task (Java `TaskEventCollectionResource.getEvents`).
    let task = load_historic_task(&engine, &id)?;
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let events = engine
        .get_history_service()
        .get_task_events(&task.id, &mut session)
        .into_iter()
        .map(task_event_response)
        .collect();
    Ok(Json(events))
}

pub(crate) async fn get_task_event(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, event_id)): Path<(String, String)>,
) -> Result<Json<TaskEventResponse>, ApiError> {
    // Get uses historic task (Java `TaskEventResource.getEvent`).
    let task = load_historic_task(&engine, &id)?;
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let event = engine
        .get_history_service()
        .get_task_event(&task.id, &event_id, &mut session)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Task '{}' event '{}' was not found",
                task.id, event_id
            ))
        })?;
    Ok(Json(task_event_response(event)))
}

/// Java `TaskEventResource.deleteEvent`: checks the *runtime* task, then
/// deletes the event (Java delegates to `taskService.deleteComment(eventId)`
/// since events and comments share a table there).
pub(crate) async fn delete_task_event(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, event_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let task = load_task(&engine, &id)?;
    engine
        .get_history_service()
        .delete_task_event(&task.id, &event_id)?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Task '{}' does not have an event with id '{}'.",
                task.id, event_id
            ))
        })?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn load_task(engine: &ProcessEngine, id: &str) -> Result<Task, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_runtime_store()
        .find_task(id, &mut session)
        .ok_or_else(|| ApiError::NotFound(format!("Task '{}' was not found", id)))
}

/// Resolve a historic task instance for comment/event list/get.
/// Java: `TaskBaseResource.getHistoricTaskFromRequest`.
fn load_historic_task(engine: &ProcessEngine, id: &str) -> Result<HistoricTaskInstance, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_runtime_store()
        .get_historic_task_instance(id, &mut session)
        .ok_or_else(|| ApiError::NotFound(format!("Task '{}' was not found", id)))
}

/// Extract authenticated user id from HTTP Basic auth (Java Authentication).
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

fn task_comment_for_historic(
    engine: &ProcessEngine,
    task: &HistoricTaskInstance,
    comment_id: &str,
) -> Result<flowable_engine::history::historic_entities::HistoricComment, ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let comment = engine
        .get_history_service()
        .get_comment(comment_id, &mut session)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Task '{}' comment '{}' was not found",
                task.id, comment_id
            ))
        })?;
    if comment.task_id.as_deref() != Some(task.id.as_str()) {
        return Err(ApiError::NotFound(format!(
            "Task '{}' comment '{}' was not found",
            task.id, comment_id
        )));
    }
    Ok(comment)
}

pub(crate) fn task_comment_response(
    comment: flowable_engine::history::historic_entities::HistoricComment,
) -> TaskCommentResponse {
    let comment_type = comment.resolved_type().to_string();
    TaskCommentResponse {
        id: comment.id.clone(),
        task_url: comment
            .task_id
            .as_ref()
            .map(|task_id| format!("/runtime/tasks/{task_id}/comments/{}", comment.id)),
        process_instance_url: comment
            .process_instance_id
            .as_ref()
            .map(|process_instance_id| {
                format!(
                    "/history/historic-process-instances/{process_instance_id}/comments/{}",
                    comment.id
                )
            }),
        message: comment.message,
        author: comment.author,
        time: comment.time.to_rfc3339(),
        task_id: comment.task_id,
        process_instance_id: comment.process_instance_id,
        comment_type,
    }
}

fn task_event_response(
    event: flowable_engine::history::historic_entities::HistoricTaskEvent,
) -> TaskEventResponse {
    let task_url = format!("/runtime/tasks/{}", event.task_id);
    TaskEventResponse {
        action: event.action,
        id: event.id.clone(),
        message: event.message,
        task_url: task_url.clone(),
        time: event.time.to_rfc3339(),
        url: format!("{task_url}/events/{}", event.id),
        user_id: event.user_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn query_variable(body: Value) -> QueryVariable {
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn task_variable_parse_accepts_all_ten_operations() {
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
            let parsed = parse_query_variable_operation(&variable).unwrap();
            assert_eq!(
                QueryVariableOperation::from_friendly_name(name),
                Some(parsed),
                "operation {name}"
            );
        }
    }

    #[test]
    fn task_variable_illegal_operation_is_400() {
        let variable = query_variable(json!({"name": "v", "operation": "bogusOp", "value": 1}));
        let error = parse_query_variable_operation(&variable).unwrap_err();
        assert!(matches!(
            error,
            ApiError::BadRequest(message) if message == "Unsupported variable query operation: bogusOp"
        ));
    }

    #[test]
    fn task_variable_nameless_non_equals_is_400() {
        let variable = query_variable(json!({"operation": "notEquals", "value": "x"}));
        let error = validate_query_variable(
            &variable,
            QueryVariableOperation::NotEquals,
            &json!("x"),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ApiError::BadRequest(message) if message ==
                "Value-only query (without a variable-name) is only supported when using 'equals' operation."
        ));
    }

    #[test]
    fn task_variable_boolean_comparison_is_400() {
        for (operation, clause, value) in [
            (QueryVariableOperation::GreaterThan, "greater than", json!(true)),
            (QueryVariableOperation::LessThan, "less than", json!(null)),
        ] {
            let variable = query_variable(json!({"name": "v", "value": value}));
            let error =
                validate_query_variable(&variable, operation, &value).unwrap_err();
            assert!(matches!(
                error,
                ApiError::BadRequest(message) if message == format!("Booleans and null cannot be used in '{clause}' condition")
            ));
        }
    }

    /// Semantic pin tests for in-memory SQL-LIKE (`%` / `_` / literal).
    /// Argument order in tasks.rs is `(pattern, value)`.
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

    #[test]
    fn task_variable_like_greater_ignore_case_positive_and_miss() {
        // like: % wildcard matches, miss otherwise.
        assert!(variable_value_matches(
            "v",
            &json!("HelloWorld"),
            Some("v"),
            QueryVariableOperation::Like,
            &json!("Hello%")
        ));
        assert!(!variable_value_matches(
            "v",
            &json!("HelloWorld"),
            Some("v"),
            QueryVariableOperation::Like,
            &json!("Nope%")
        ));
        // greaterThan: numeric, miss on equal.
        assert!(variable_value_matches(
            "n",
            &json!(10),
            Some("n"),
            QueryVariableOperation::GreaterThan,
            &json!(5)
        ));
        assert!(!variable_value_matches(
            "n",
            &json!(10),
            Some("n"),
            QueryVariableOperation::GreaterThan,
            &json!(10)
        ));
        // equalsIgnoreCase: miss on differing case-insensitive value.
        assert!(variable_value_matches(
            "s",
            &json!("Hello"),
            Some("s"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("hello")
        ));
        assert!(!variable_value_matches(
            "s",
            &json!("Hello"),
            Some("s"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("world")
        ));
        // Expected-name mismatch never matches.
        assert!(!variable_value_matches(
            "s",
            &json!("Hello"),
            Some("other"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("hello")
        ));
    }
}
