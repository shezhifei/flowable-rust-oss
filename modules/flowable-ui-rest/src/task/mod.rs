//! Task UI app (`/app/rest/**`) — stream B aggregation over the process engine.
//!
//! Prefer in-process engine APIs (not HTTP self-calls). Mount with
//! [`router_with_engine`] when an engine is available; [`router`] is the
//! no-engine scaffold used by stream-A-style `ui_router()` merges.

mod rest_variable;

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use flowable_cmmn_engine::CmmnCaseInstanceStartRequest;
use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query as EngineQuery;
use flowable_engine::engine::task_service::TaskUpdate;
use flowable_engine::identity::entities::User;
use flowable_engine::runtime::process_instance_builder::ProcessInstanceBuilder;
use flowable_engine::task::Task;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub use rest_variable::{
    create_rest_variable, rest_variable_value, RestVariable, RestVariableScope,
};

use crate::auth::UiAuth;

fn default_user_id() -> String {
    std::env::var("FLOWABLE_UI_DEFAULT_USER").unwrap_or_else(|_| "admin".into())
}

/// The user a handler acts as: the authenticated session when the UI auth
/// middleware ran (always the case in enforced deployments), else the
/// development fallback used by tests that mount this router without the auth
/// layer. Acting as a fixed "admin" regardless of session would let any
/// authenticated user claim, start, and comment in another user's name.
fn effective_user_id(auth: Option<&UiAuth>) -> String {
    auth.map(|a| a.user_id().to_string())
        .unwrap_or_else(default_user_id)
}

/// Test helper: full task router with an in-process engine extension.
pub fn router_with_engine(engine: Arc<ProcessEngine>) -> Router {
    router().layer(Extension(engine))
}

/// Task aggregation router. Engine comes from `Extension<Arc<ProcessEngine>>`
/// (stream A / `flowable-rest` already layers it on the app).
pub fn router() -> Router {
    Router::new()
        .route("/app/rest/health", get(health))
        // Account (Java flowable-ui-task AccountResource; the workflow app
        // bootstraps its filters and assignment logic off this representation)
        .route("/app/rest/account", get(account))
        // Tasks
        .route("/app/rest/tasks", post(create_task))
        .route(
            "/app/rest/tasks/:task_id",
            get(get_task).put(update_task),
        )
        .route("/app/rest/tasks/:task_id/subtasks", get(list_subtasks))
        .route("/app/rest/query/tasks", post(query_tasks))
        .route("/app/rest/query/history/tasks", post(query_historic_tasks))
        .route(
            "/app/rest/tasks/:task_id/action/complete",
            put(action_complete),
        )
        .route("/app/rest/tasks/:task_id/action/assign", put(action_assign))
        .route("/app/rest/tasks/:task_id/action/claim", put(action_claim))
        .route(
            "/app/rest/tasks/:task_id/action/involve",
            put(action_involve),
        )
        .route(
            "/app/rest/tasks/:task_id/action/remove-involved",
            put(action_remove_involved),
        )
        // Forms (minimal)
        .route(
            "/app/rest/task-forms/:task_id",
            get(get_task_form).post(complete_task_form),
        )
        .route(
            "/app/rest/task-forms/:task_id/save-form",
            post(save_task_form),
        )
        // Comments
        .route(
            "/app/rest/tasks/:task_id/comments",
            get(list_task_comments).post(add_task_comment),
        )
        .route(
            "/app/rest/process-instances/:process_instance_id/comments",
            get(list_pi_comments).post(add_pi_comment),
        )
        // Process
        .route(
            "/app/rest/process-instances",
            post(start_process_instance),
        )
        .route(
            "/app/rest/process-instances/:process_instance_id",
            get(get_process_instance).delete(delete_process_instance),
        )
        .route(
            "/app/rest/process-instances/:process_instance_id/start-form",
            get(process_instance_start_form),
        )
        .route(
            "/app/rest/query/process-instances",
            post(query_process_instances),
        )
        .route(
            "/app/rest/process-definitions",
            get(list_process_definitions),
        )
        .route(
            "/app/rest/process-definitions/:process_definition_id/start-form",
            get(process_definition_start_form),
        )
        // Case
        .route("/app/rest/case-definitions", get(list_case_definitions))
        .route("/app/rest/case-instances", post(start_case_instance))
        .route(
            "/app/rest/case-instances/:case_instance_id",
            get(get_case_instance).delete(delete_case_instance),
        )
        .route(
            "/app/rest/query/case-instances",
            post(query_case_instances),
        )
        // Debugger (gated by FLOWABLE_EXPERIMENTAL_DEBUGGER_ENABLED)
        .route("/app/rest/debugger", get(debugger_allowed))
        .route(
            "/app/rest/debugger/breakpoints",
            get(list_breakpoints)
                .post(add_breakpoint)
                .delete(remove_breakpoint),
        )
        .route(
            "/app/rest/debugger/eventlog/:process_instance_id",
            get(debugger_event_log),
        )
        .route(
            "/app/rest/debugger/executions/:process_instance_id",
            get(debugger_executions),
        )
        .route(
            "/app/rest/debugger/variables/:execution_id",
            get(debugger_variables),
        )
        // Workflow users/groups
        .route("/app/rest/workflow-users", get(workflow_users))
        .route("/app/rest/workflow-groups", get(workflow_groups))
        .route("/app/rest/workflow-groups/:group_id", get(workflow_group))
        .route("/app/rest/users/:user_id", get(get_user))
        // App definitions
        .route(
            "/app/rest/runtime/app-definitions",
            get(list_app_definitions),
        )
        .route(
            "/app/rest/runtime/app-definitions/:app_definition_key",
            get(get_app_definition),
        )
        // Related content
        .route(
            "/app/rest/tasks/:task_id/content",
            get(list_task_content).post(add_task_content),
        )
        .route(
            "/app/rest/process-instances/:process_instance_id/content",
            get(list_pi_content).post(add_pi_content),
        )
        .route(
            "/app/rest/case-instances/:case_instance_id/content",
            get(list_case_content),
        )
        .route(
            "/app/rest/content/:content_id",
            get(get_content).delete(delete_content),
        )
}


// ---------------------------------------------------------------------------
// Models (UI JSON shapes)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultListDataRepresentation<T> {
    pub size: i32,
    pub total: i64,
    pub start: i32,
    pub data: Vec<T>,
}

impl<T> ResultListDataRepresentation<T> {
    fn from_page(data: Vec<T>, start: i32, total: Option<i64>) -> Self {
        let size = data.len() as i32;
        let total = total.unwrap_or(size as i64);
        Self {
            size,
            total,
            start,
            data,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UserRepresentation {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

impl From<&User> for UserRepresentation {
    fn from(u: &User) -> Self {
        let full = match (&u.first_name, &u.last_name) {
            (Some(f), Some(l)) => Some(format!("{f} {l}")),
            (Some(f), None) => Some(f.clone()),
            (None, Some(l)) => Some(l.clone()),
            _ => Some(u.id.clone()),
        };
        Self {
            id: u.id.clone(),
            first_name: u.first_name.clone(),
            last_name: u.last_name.clone(),
            email: u.email.clone(),
            full_name: full,
            tenant_id: u.tenant_id.clone(),
        }
    }
}

impl UserRepresentation {
    fn from_id(id: &str) -> Self {
        Self {
            id: id.to_string(),
            first_name: None,
            last_name: None,
            email: None,
            full_name: Some(id.to_string()),
            tenant_id: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskRepresentation {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<UserRepresentation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_instance_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_definition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_definition_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_definition_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_key: Option<String>,
    #[serde(default)]
    pub initiator_can_complete_task: bool,
    #[serde(default)]
    pub is_member_of_candidate_group: bool,
    #[serde(default)]
    pub is_member_of_candidate_users: bool,
}

fn task_to_rep(engine: &ProcessEngine, task: &Task) -> TaskRepresentation {
    let assignee = task
        .assignee
        .as_ref()
        .map(|id| resolve_user(engine, id));
    let (pd_id, pd_name, pd_key) = if task.process_instance_id.is_empty() {
        (None, None, None)
    } else {
        // Best-effort: leave definition enrichment empty if lookup fails.
        (None, None, None)
    };
    let _ = pd_id;
    TaskRepresentation {
        id: task.id.clone(),
        name: task.name.clone(),
        description: task.description.clone(),
        category: task.category.clone(),
        assignee,
        created: task.created_time.map(|t| t.to_rfc3339()),
        due_date: task.due_date.map(|t| t.to_rfc3339()),
        end_date: task.completed_time.map(|t| t.to_rfc3339()),
        priority: task.priority,
        process_instance_id: if task.process_instance_id.is_empty() {
            None
        } else {
            Some(task.process_instance_id.clone())
        },
        process_instance_name: None,
        process_definition_id: pd_id,
        process_definition_name: pd_name,
        process_definition_key: pd_key,
        parent_task_id: task.parent_task_id.clone(),
        form_key: task.form_key.clone(),
        initiator_can_complete_task: false,
        is_member_of_candidate_group: false,
        is_member_of_candidate_users: false,
    }
}

fn resolve_user(engine: &ProcessEngine, id: &str) -> UserRepresentation {
    let users = engine
        .get_identity_service()
        .create_user_query()
        .list()
        .unwrap_or_default();
    users
        .iter()
        .find(|u| u.id == id)
        .map(UserRepresentation::from)
        .unwrap_or_else(|| UserRepresentation::from_id(id))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health(Extension(_engine): Extension<Arc<ProcessEngine>>) -> Json<Value> {
    Json(json!({ "status": "ok", "app": "task", "engine": true }))
}

/// `GET /app/rest/account` — Java flowable-ui-task `AccountResource.getAccount`.
///
/// The workflow app resolves the current user here before issuing any task
/// query (`data.assignee = account.id`), so without this route the task list
/// never loads in enforced mode. Shape mirrors the idm app's account
/// representation: user fields plus group memberships and effective privilege
/// names.
async fn account(
    auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<Value>, TaskError> {
    let identity = engine.get_identity_service();
    let user = identity
        .find_user_by_id(auth.user_id())
        .ok_or_else(|| TaskError::not_found("Account not found".to_string()))?;
    let full_name = format!(
        "{} {}",
        user.first_name.clone().unwrap_or_default(),
        user.last_name.clone().unwrap_or_default()
    );
    let groups: Vec<Value> = identity
        .get_groups_by_user(&user.id)
        .into_iter()
        .map(|group| json!({ "id": group.id, "name": group.name, "type": group.group_type }))
        .collect();
    let mut privileges: Vec<String> = identity
        .get_privileges_for_user(&user.id)
        .into_iter()
        .map(|privilege| privilege.name)
        .collect();
    privileges.sort();
    privileges.dedup();
    Ok(Json(json!({
        "id": user.id,
        "firstName": user.first_name,
        "lastName": user.last_name,
        "email": user.email,
        "fullName": full_name,
        "tenantId": user.tenant_id,
        "groups": groups,
        "privileges": privileges,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTaskBody {
    name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    assignee: Option<String>,
    parent_task_id: Option<String>,
}

async fn create_task(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(body): Json<CreateTaskBody>,
) -> Result<impl IntoResponse, TaskError> {
    let name = body
        .name
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskError::bad_request("Task name is required"))?;
    let mut task = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        name,
    );
    task.description = body.description;
    task.category = body.category;
    task.parent_task_id = body.parent_task_id;
    task.assignee = Some(
        body.assignee
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| effective_user_id(auth.as_ref())),
    );
    let created = engine
        .get_task_service()
        .create_task(task)
        .map_err(TaskError::from_engine)?;
    Ok(Json(task_to_rep(&engine, &created)))
}

async fn get_task(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let task = find_task(&engine, &task_id)?;
    Ok(Json(task_to_rep(&engine, &task)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTaskBody {
    name: Option<String>,
    description: Option<String>,
    assignee: Option<String>,
    due_date: Option<String>,
    priority: Option<i32>,
    category: Option<String>,
    form_key: Option<String>,
}

async fn update_task(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
    Json(body): Json<UpdateTaskBody>,
) -> Result<impl IntoResponse, TaskError> {
    let due = body
        .due_date
        .as_deref()
        .map(parse_date)
        .transpose()
        .map_err(TaskError::bad_request)?;
    let update = TaskUpdate {
        name: body.name,
        description: body.description.map(Some),
        assignee: body.assignee.map(Some),
        owner: None,
        delegation_state: None,
        parent_task_id: None,
        priority: body.priority.map(Some),
        due_date: due.map(Some),
        category: body.category.map(Some),
        form_key: body.form_key.map(Some),
        tenant_id: None,
    };
    let task = engine
        .get_task_service()
        .update_task_by_id(task_id, update)
        .map_err(TaskError::from_engine)?;
    Ok(Json(task_to_rep(&engine, &task)))
}

async fn list_subtasks(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let tasks = engine
        .get_task_service()
        .get_sub_tasks(task_id)
        .map_err(TaskError::from_engine)?;
    let data: Vec<_> = tasks
        .iter()
        .map(|t| task_to_rep(&engine, t))
        .collect();
    Ok(Json(ResultListDataRepresentation::from_page(data, 0, None)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskQueryBody {
    state: Option<String>,
    text: Option<String>,
    assignment: Option<String>,
    process_instance_id: Option<String>,
    #[allow(dead_code)]
    process_definition_id: Option<String>,
    due_before: Option<String>,
    due_after: Option<String>,
    sort: Option<String>,
    page: Option<i32>,
    size: Option<i32>,
    include_process_instance: Option<bool>,
}

async fn query_tasks(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(body): Json<TaskQueryBody>,
) -> Result<impl IntoResponse, TaskError> {
    // "completed" historic path uses the same active query filtered by is_completed for now.
    list_tasks_internal(&engine, body, false, effective_user_id(auth.as_ref()))
}

async fn query_historic_tasks(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(body): Json<TaskQueryBody>,
) -> Result<impl IntoResponse, TaskError> {
    list_tasks_internal(&engine, body, true, effective_user_id(auth.as_ref()))
}

fn list_tasks_internal(
    engine: &ProcessEngine,
    body: TaskQueryBody,
    historic: bool,
    user_id: String,
) -> Result<Json<ResultListDataRepresentation<TaskRepresentation>>, TaskError> {
    let page = body.page.unwrap_or(0).max(0) as usize;
    let size = body.size.unwrap_or(25).clamp(1, 1000) as usize;

    let mut q = engine.get_task_service().create_task_query();
    if let Some(pi) = body.process_instance_id.filter(|s| !s.is_empty()) {
        q = q.process_instance_id(pi);
    }
    if let Some(assignee_mode) = body.assignment.as_deref() {
        match assignee_mode {
            "assignee" => q = q.task_assignee(user_id.clone()),
            "candidate" => q = q.task_candidate_user(user_id.clone()),
            other if other.starts_with("group_") => {
                let gid = other.trim_start_matches("group_");
                q = q.task_candidate_group(gid.to_string());
            }
            _ => {
                // involved — approximate with candidate user + assignee filters via post-filter
            }
        }
    }
    let mut tasks = q.list().map_err(TaskError::from_engine)?;

    if historic || body.state.as_deref() == Some("completed") {
        tasks.retain(|t| t.is_completed);
    } else {
        tasks.retain(|t| !t.is_completed);
    }
    if let Some(text) = body.text.filter(|s| !s.is_empty()) {
        let lower = text.to_lowercase();
        tasks.retain(|t| t.name.to_lowercase().contains(&lower));
    }
    if let Some(due_before) = body.due_before.as_deref() {
        if let Ok(dt) = parse_date(due_before) {
            tasks.retain(|t| t.due_date.map(|d| d < dt).unwrap_or(false));
        }
    }
    if let Some(due_after) = body.due_after.as_deref() {
        if let Ok(dt) = parse_date(due_after) {
            tasks.retain(|t| t.due_date.map(|d| d > dt).unwrap_or(false));
        }
    }
    match body.sort.as_deref() {
        Some("created-asc") => tasks.sort_by_key(|t| t.created_time),
        Some("due-asc") => tasks.sort_by_key(|t| t.due_date),
        Some("due-desc") => {
            tasks.sort_by(|a, b| b.due_date.cmp(&a.due_date));
        }
        _ => {
            // created-desc default
            tasks.sort_by(|a, b| b.created_time.cmp(&a.created_time));
        }
    }

    let total = tasks.len() as i64;
    let start = page * size;
    let page_tasks: Vec<_> = tasks.into_iter().skip(start).take(size).collect();
    let mut data: Vec<_> = page_tasks
        .iter()
        .map(|t| task_to_rep(&engine, t))
        .collect();

    if body.include_process_instance == Some(true) {
        // Placeholder names; full PI name lookup can be filled later.
        for rep in &mut data {
            if rep.process_instance_name.is_none() {
                rep.process_instance_name = rep.process_instance_id.clone();
            }
        }
    }

    Ok(Json(ResultListDataRepresentation::from_page(
        data,
        start as i32,
        Some(total),
    )))
}

async fn action_complete(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let _ = find_task(&engine, &task_id)?;
    engine
        .get_task_service()
        .complete_task_by_id(task_id)
        .map_err(TaskError::from_engine)?;
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignBody {
    assignee: Option<String>,
}

async fn action_assign(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
    Json(body): Json<AssignBody>,
) -> Result<impl IntoResponse, TaskError> {
    let assignee = body
        .assignee
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskError::bad_request("Assignee is required"))?;
    let update = TaskUpdate {
        assignee: Some(Some(assignee)),
        ..Default::default()
    };
    let task = engine
        .get_task_service()
        .update_task_by_id(task_id, update)
        .map_err(TaskError::from_engine)?;
    Ok(Json(task_to_rep(&engine, &task)))
}

async fn action_claim(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let user = effective_user_id(auth.as_ref());
    engine
        .get_task_service()
        .claim_task_by_id(task_id, user)
        .map_err(TaskError::from_engine)?;
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvolveBody {
    user_id: Option<String>,
    email: Option<String>,
}

async fn action_involve(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
    Json(body): Json<InvolveBody>,
) -> Result<impl IntoResponse, TaskError> {
    let _ = find_task(&engine, &task_id)?;
    let user_id = body
        .user_id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskError::bad_request("User id is required"))?;
    engine
        .get_task_service()
        .add_identity_link(task_id, Some(user_id), None, "participant".into())
        .map_err(TaskError::from_engine)?;
    Ok(StatusCode::OK)
}

async fn action_remove_involved(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
    Json(body): Json<InvolveBody>,
) -> Result<impl IntoResponse, TaskError> {
    let _ = find_task(&engine, &task_id)?;
    let user_id = body
        .user_id
        .or(body.email)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskError::bad_request("User id or email is required"))?;
    engine
        .get_task_service()
        .delete_identity_link(task_id, Some(user_id), None, "participant".into())
        .map_err(TaskError::from_engine)?;
    Ok(StatusCode::OK)
}

async fn get_task_form(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let task = find_task(&engine, &task_id)?;
    // Minimal form info — full form-service assembly is progressive.
    Ok(Json(json!({
        "id": task.form_key,
        "name": task.name,
        "key": task.form_key,
        "fields": [],
        "outcomes": [{ "id": "complete", "name": "Complete" }]
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompleteFormBody {
    form_id: Option<String>,
    outcome: Option<String>,
    values: Option<HashMap<String, Value>>,
}

async fn complete_task_form(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
    Json(body): Json<CompleteFormBody>,
) -> Result<impl IntoResponse, TaskError> {
    let _ = body.form_id;
    let _ = body.outcome;
    let vars = body.values.unwrap_or_default();
    if vars.is_empty() {
        engine
            .get_task_service()
            .complete_task_by_id(task_id)
            .map_err(TaskError::from_engine)?;
    } else {
        engine
            .get_task_service()
            .complete_task_by_id_with_variables(task_id, vars)
            .map_err(TaskError::from_engine)?;
    }
    Ok(StatusCode::OK)
}

async fn save_task_form(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
    Json(body): Json<CompleteFormBody>,
) -> Result<impl IntoResponse, TaskError> {
    let _ = find_task(&engine, &task_id)?;
    if let Some(values) = body.values {
        for (k, v) in values {
            engine
                .get_task_service()
                .set_task_local_variable(task_id.clone(), k, v)
                .map_err(TaskError::from_engine)?;
        }
    }
    Ok(StatusCode::OK)
}

// ---- Comments ----

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentRepresentation {
    id: Option<String>,
    message: Option<String>,
    created: Option<String>,
    created_by: Option<UserRepresentation>,
}

async fn list_task_comments(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let _ = find_task(&engine, &task_id)?;
    let mut session = engine
        .get_runtime_store()
        .create_session()
        .map_err(|e| TaskError::internal(e.to_string()))?;
    let comments = engine
        .get_history_service()
        .get_task_comments(&task_id, &mut session);
    let data: Vec<_> = comments
        .into_iter()
        .map(|c| CommentRepresentation {
            id: Some(c.id.clone()),
            message: Some(c.message.clone()),
            created: Some(c.time.to_rfc3339()),
            created_by: c.author.as_ref().map(|u| resolve_user(&engine, u)),
        })
        .collect();
    Ok(Json(ResultListDataRepresentation::from_page(data, 0, None)))
}

async fn add_task_comment(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
    Json(body): Json<CommentRepresentation>,
) -> Result<impl IntoResponse, TaskError> {
    let task = find_task(&engine, &task_id)?;
    let message = body
        .message
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskError::bad_request("Comment message is required"))?;
    let user_id = effective_user_id(auth.as_ref());
    let pi = if task.process_instance_id.is_empty() {
        None
    } else {
        Some(task.process_instance_id.as_str())
    };
    let comment = engine
        .get_history_service()
        .create_task_comment(&task_id, pi, &message, Some(&user_id))
        .map_err(TaskError::from_engine)?;
    Ok(Json(CommentRepresentation {
        id: Some(comment.id),
        message: Some(comment.message),
        created: Some(comment.time.to_rfc3339()),
        created_by: Some(resolve_user(&engine, &user_id)),
    }))
}

async fn list_pi_comments(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let mut session = engine
        .get_runtime_store()
        .create_session()
        .map_err(|e| TaskError::internal(e.to_string()))?;
    let comments = engine
        .get_history_service()
        .get_process_instance_comments(&process_instance_id, &mut session);
    let data: Vec<_> = comments
        .into_iter()
        .map(|c| CommentRepresentation {
            id: Some(c.id.clone()),
            message: Some(c.message.clone()),
            created: Some(c.time.to_rfc3339()),
            created_by: c.author.as_ref().map(|u| resolve_user(&engine, u)),
        })
        .collect();
    Ok(Json(ResultListDataRepresentation::from_page(data, 0, None)))
}

async fn add_pi_comment(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    Json(body): Json<CommentRepresentation>,
) -> Result<impl IntoResponse, TaskError> {
    let message = body
        .message
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskError::bad_request("Comment message is required"))?;
    let user_id = effective_user_id(auth.as_ref());
    let comment = engine
        .get_history_service()
        .create_process_instance_comment(&process_instance_id, &message, Some(&user_id))
        .map_err(TaskError::from_engine)?;
    Ok(Json(CommentRepresentation {
        id: Some(comment.id),
        message: Some(comment.message),
        created: Some(comment.time.to_rfc3339()),
        created_by: Some(resolve_user(&engine, &user_id)),
    }))
}

// ---- Process ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartProcessBody {
    process_definition_id: Option<String>,
    process_definition_key: Option<String>,
    name: Option<String>,
    business_key: Option<String>,
    values: Option<HashMap<String, Value>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessInstanceRepresentation {
    id: String,
    name: Option<String>,
    business_key: Option<String>,
    process_definition_id: Option<String>,
    ended: bool,
    started: Option<String>,
    started_by: Option<UserRepresentation>,
}

fn list_all_process_instances(
    engine: &ProcessEngine,
) -> Result<Vec<flowable_engine::runtime::process_instance::ProcessInstance>, TaskError> {
    engine
        .get_runtime_store()
        .db_store()
        .find_all::<flowable_engine::runtime::process_instance::ProcessInstance>(
            "process_instances",
        )
        .map_err(|e| TaskError::internal(e.to_string()))
}

async fn start_process_instance(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(body): Json<StartProcessBody>,
) -> Result<impl IntoResponse, TaskError> {
    let user_id = effective_user_id(auth.as_ref());
    let mut builder = ProcessInstanceBuilder::new().start_user_id(user_id.clone());
    if let Some(id) = body.process_definition_id.filter(|s| !s.is_empty()) {
        builder = builder.process_definition_id(id);
    } else if let Some(key) = body.process_definition_key.filter(|s| !s.is_empty()) {
        builder = builder.process_definition_key(key);
    } else {
        return Err(TaskError::bad_request(
            "processDefinitionId or processDefinitionKey is required",
        ));
    }
    if let Some(name) = body.name {
        builder = builder.name(name);
    }
    if let Some(bk) = body.business_key {
        builder = builder.business_key(bk);
    }
    if let Some(values) = body.values {
        for (k, v) in values {
            builder = builder.variable(k, v);
        }
    }
    let pi = engine
        .get_runtime_service()
        .start_process_instance(builder)
        .map_err(TaskError::from_engine)?;
    Ok(Json(ProcessInstanceRepresentation {
        id: pi.id.clone(),
        name: pi.name.clone(),
        business_key: pi.business_key.clone(),
        process_definition_id: Some(pi.process_definition_id.clone()),
        ended: pi.is_ended,
        started: pi.start_time.map(|t| t.to_rfc3339()),
        started_by: Some(resolve_user(&engine, &user_id)),
    }))
}

async fn get_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let pi = list_all_process_instances(&engine)?
        .into_iter()
        .find(|p| p.id == process_instance_id)
        .ok_or_else(|| TaskError::not_found(format!("Process instance {process_instance_id}")))?;
    Ok(Json(ProcessInstanceRepresentation {
        id: pi.id.clone(),
        name: pi.name.clone(),
        business_key: pi.business_key.clone(),
        process_definition_id: Some(pi.process_definition_id.clone()),
        ended: pi.is_ended,
        started: pi.start_time.map(|t| t.to_rfc3339()),
        started_by: pi
            .start_user_id
            .as_ref()
            .map(|u| resolve_user(&engine, u)),
    }))
}

async fn delete_process_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    engine
        .get_runtime_service()
        .bulk_delete_process_instances(vec![process_instance_id], Some("Deleted via task UI".into()))
        .map_err(TaskError::from_engine)?;
    Ok(StatusCode::OK)
}

async fn process_instance_start_form(
    Extension(_engine): Extension<Arc<ProcessEngine>>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    Json(json!({ "id": null, "fields": [], "outcomes": [] }))
}

async fn query_process_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, TaskError> {
    let page = body.get("page").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
    let size = body
        .get("size")
        .and_then(|v| v.as_i64())
        .unwrap_or(25)
        .clamp(1, 1000) as usize;
    let mut list = list_all_process_instances(&engine)?;
    if let Some(key) = body
        .get("processDefinitionKey")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        list.retain(|pi| {
            pi.process_definition_key == key || pi.process_definition_id.contains(key)
        });
    }
    let total = list.len() as i64;
    let start = page * size;
    let data: Vec<_> = list
        .into_iter()
        .skip(start)
        .take(size)
        .map(|pi| ProcessInstanceRepresentation {
            id: pi.id.clone(),
            name: pi.name.clone(),
            business_key: pi.business_key.clone(),
            process_definition_id: Some(pi.process_definition_id.clone()),
            ended: pi.is_ended,
            started: pi.start_time.map(|t| t.to_rfc3339()),
            started_by: pi
                .start_user_id
                .as_ref()
                .map(|u| resolve_user(&engine, u)),
        })
        .collect();
    Ok(Json(ResultListDataRepresentation::from_page(
        data,
        start as i32,
        Some(total),
    )))
}

async fn list_process_definitions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, TaskError> {
    let latest = params
        .get("latest")
        .map(|v| v == "true")
        .unwrap_or(true);
    let defs = engine
        .get_repository_service()
        .get_process_definitions()
        .map_err(TaskError::from_engine)?;
    let mut data: Vec<Value> = defs
        .into_iter()
        .filter(|d| !latest || d.version > 0) // all versions for now; latest filter soft
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "key": d.key,
                "version": d.version,
                "category": d.category,
                "deploymentId": d.deployment_id,
                "description": d.description,
                "hasStartFormKey": d.has_start_form_key,
            })
        })
        .collect();
    // Prefer highest version per key when latest=true.
    if latest {
        let mut best: HashMap<String, Value> = HashMap::new();
        for d in data {
            let key = d["key"].as_str().unwrap_or("").to_string();
            let ver = d["version"].as_i64().unwrap_or(0);
            let replace = match best.get(&key) {
                Some(existing) => ver >= existing["version"].as_i64().unwrap_or(0),
                None => true,
            };
            if replace {
                best.insert(key, d);
            }
        }
        data = best.into_values().collect();
    }
    let total = data.len() as i64;
    Ok(Json(ResultListDataRepresentation::from_page(
        data, 0, Some(total),
    )))
}

async fn process_definition_start_form(
    Extension(_engine): Extension<Arc<ProcessEngine>>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    Json(json!({ "id": null, "fields": [], "outcomes": [] }))
}

// ---- Case ----

fn cmmn_engine(
    engine: &ProcessEngine,
) -> Result<Arc<flowable_cmmn_engine::CmmnEngine>, TaskError> {
    engine
        .get_config()
        .cmmn_engine
        .clone()
        .ok_or_else(|| TaskError::bad_request("CMMN engine is not configured on this process engine"))
}

async fn list_case_definitions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<impl IntoResponse, TaskError> {
    let cmmn = cmmn_engine(&engine)?;
    let defs = cmmn
        .repository_service()
        .create_case_definition_query()
        .list()
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    let data: Vec<_> = defs
        .into_iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "key": d.key,
                "version": d.version,
                "category": d.category,
                "deploymentId": d.deployment_id,
            })
        })
        .collect();
    let total = data.len() as i64;
    Ok(Json(ResultListDataRepresentation::from_page(
        data, 0, Some(total),
    )))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartCaseBody {
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    name: Option<String>,
    business_key: Option<String>,
    values: Option<HashMap<String, Value>>,
}

async fn start_case_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(body): Json<StartCaseBody>,
) -> Result<impl IntoResponse, TaskError> {
    let cmmn = cmmn_engine(&engine)?;
    let mut request = CmmnCaseInstanceStartRequest::default();
    request.name = body.name;
    request.business_key = body.business_key;
    if let Some(vars) = body.values {
        request.variables = Value::Object(vars.into_iter().collect());
    }
    let instance = if let Some(id) = body.case_definition_id.filter(|s| !s.is_empty()) {
        cmmn.runtime_service()
            .start_case_instance_by_id(&id, request)
            .map_err(|e| TaskError::bad_request(e.to_string()))?
    } else if let Some(key) = body.case_definition_key.filter(|s| !s.is_empty()) {
        cmmn.runtime_service()
            .start_case_instance_by_key(&key, request)
            .map_err(|e| TaskError::bad_request(e.to_string()))?
    } else {
        return Err(TaskError::bad_request(
            "caseDefinitionId or caseDefinitionKey is required",
        ));
    };
    Ok(Json(json!({
        "id": instance.id,
        "name": instance.name,
        "businessKey": instance.business_key,
        "caseDefinitionId": instance.case_definition_id,
        "caseDefinitionKey": instance.case_definition_key,
        "ended": instance.ended_at.is_some(),
    })))
}

async fn get_case_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(case_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let cmmn = cmmn_engine(&engine)?;
    let list = cmmn
        .runtime_service()
        .create_case_instance_query()
        .list()
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    let instance = list
        .into_iter()
        .find(|c| c.id == case_instance_id)
        .ok_or_else(|| TaskError::not_found(format!("Case instance {case_instance_id}")))?;
    Ok(Json(json!({
        "id": instance.id,
        "name": instance.name,
        "businessKey": instance.business_key,
        "caseDefinitionId": instance.case_definition_id,
        "caseDefinitionKey": instance.case_definition_key,
        "ended": instance.ended_at.is_some(),
    })))
}

async fn delete_case_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(case_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let cmmn = cmmn_engine(&engine)?;
    cmmn.runtime_service()
        .terminate_case_instance(&case_instance_id)
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    Ok(StatusCode::OK)
}

async fn query_case_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, TaskError> {
    let cmmn = cmmn_engine(&engine)?;
    let page = body.get("page").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
    let size = body
        .get("size")
        .and_then(|v| v.as_i64())
        .unwrap_or(25)
        .clamp(1, 1000) as usize;
    let mut list = cmmn
        .runtime_service()
        .create_case_instance_query()
        .list()
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    if let Some(key) = body
        .get("caseDefinitionKey")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        list.retain(|c| c.case_definition_key == key);
    }
    let total = list.len() as i64;
    let start = page * size;
    let data: Vec<_> = list
        .into_iter()
        .skip(start)
        .take(size)
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "businessKey": c.business_key,
                "caseDefinitionId": c.case_definition_id,
                "caseDefinitionKey": c.case_definition_key,
                "ended": c.ended_at.is_some(),
            })
        })
        .collect();
    Ok(Json(ResultListDataRepresentation::from_page(
        data,
        start as i32,
        Some(total),
    )))
}

// ---- Content ----

fn content_service(engine: Arc<ProcessEngine>) -> FlowableContentService {
    FlowableContentService::new(engine)
}

async fn list_task_content(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let svc = content_service(engine);
    let items = svc
        .create_content_item_query()
        .task_id(task_id)
        .list()
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    let data: Vec<_> = items.into_iter().map(content_item_json).collect();
    Ok(Json(ResultListDataRepresentation::from_page(data, 0, None)))
}

async fn list_pi_content(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let svc = content_service(engine);
    let items = svc
        .create_content_item_query()
        .process_instance_id(process_instance_id)
        .list()
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    let data: Vec<_> = items.into_iter().map(content_item_json).collect();
    Ok(Json(ResultListDataRepresentation::from_page(data, 0, None)))
}

async fn list_case_content(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(case_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let svc = content_service(engine);
    let items = svc
        .create_content_item_query()
        .scope_id(case_instance_id)
        .scope_type("cmmn")
        .list()
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    let data: Vec<_> = items.into_iter().map(content_item_json).collect();
    Ok(Json(ResultListDataRepresentation::from_page(data, 0, None)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContentBody {
    name: Option<String>,
    content: Option<String>,
    mime_type: Option<String>,
}

async fn add_task_content(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(task_id): Path<String>,
    Json(body): Json<ContentBody>,
) -> Result<impl IntoResponse, TaskError> {
    let name = body
        .name
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskError::bad_request("Content name is required"))?;
    let svc = content_service(engine);
    let item = svc
        .create_content_item(CreateContentItemRequest {
            name,
            mime_type: body.mime_type,
            description: None,
            attachment_type: None,
            external_url: None,
            content: body.content,
            task_id: Some(task_id),
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: Some(effective_user_id(auth.as_ref())),
            expires_in_seconds: None,
        })
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    Ok(Json(content_item_json(item)))
}

async fn add_pi_content(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    Json(body): Json<ContentBody>,
) -> Result<impl IntoResponse, TaskError> {
    let name = body
        .name
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskError::bad_request("Content name is required"))?;
    let svc = content_service(engine);
    let item = svc
        .create_content_item(CreateContentItemRequest {
            name,
            mime_type: body.mime_type,
            description: None,
            attachment_type: None,
            external_url: None,
            content: body.content,
            task_id: None,
            process_instance_id: Some(process_instance_id),
            scope_type: None,
            scope_id: None,
            created_by: Some(effective_user_id(auth.as_ref())),
            expires_in_seconds: None,
        })
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    Ok(Json(content_item_json(item)))
}

async fn get_content(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(content_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let svc = content_service(engine);
    let item = svc
        .get_content_item(&content_id)
        .map_err(|e| {
            let s = e.to_string();
            if s.to_lowercase().contains("not found") {
                TaskError::not_found(format!("Content {content_id}"))
            } else {
                TaskError::bad_request(s)
            }
        })?;
    Ok(Json(content_item_json(item)))
}

async fn delete_content(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(content_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let svc = content_service(engine);
    svc.delete_content_item(&content_id)
        .map_err(|e| TaskError::bad_request(e.to_string()))?;
    Ok(StatusCode::OK)
}

fn content_item_json(item: flowable_content_service::ContentItem) -> Value {
    json!({
        "id": item.id,
        "name": item.name,
        "mimeType": item.mime_type,
        "contentAvailable": item.content.is_some() || item.storage_id.is_some(),
        "contentSize": item.content_size,
        "created": item.created_at,
        "createdBy": item.created_by,
        "taskId": item.task_id,
        "processInstanceId": item.process_instance_id,
        "scopeId": item.scope_id,
        "scopeType": item.scope_type,
    })
}

// ---- Debugger ----

fn debugger_enabled() -> bool {
    std::env::var("FLOWABLE_EXPERIMENTAL_DEBUGGER_ENABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

async fn debugger_allowed() -> Json<bool> {
    Json(debugger_enabled())
}

async fn list_breakpoints() -> Result<impl IntoResponse, TaskError> {
    if !debugger_enabled() {
        return Err(TaskError::bad_request(
            "property flowable.experimental.debugger.enabled is not enabled",
        ));
    }
    // In-memory breakpoints live in a process-local static.
    Ok(Json(DEBUG_BREAKPOINTS.lock().unwrap().clone()))
}

async fn add_breakpoint(Json(body): Json<Value>) -> Result<impl IntoResponse, TaskError> {
    if !debugger_enabled() {
        return Err(TaskError::bad_request(
            "property flowable.experimental.debugger.enabled is not enabled",
        ));
    }
    DEBUG_BREAKPOINTS.lock().unwrap().push(body);
    Ok(StatusCode::OK)
}

async fn remove_breakpoint(Json(body): Json<Value>) -> Result<impl IntoResponse, TaskError> {
    if !debugger_enabled() {
        return Err(TaskError::bad_request(
            "property flowable.experimental.debugger.enabled is not enabled",
        ));
    }
    let mut guard = DEBUG_BREAKPOINTS.lock().unwrap();
    guard.retain(|b| b != &body);
    Ok(StatusCode::OK)
}

async fn debugger_event_log(
    Extension(_engine): Extension<Arc<ProcessEngine>>,
    Path(_process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    if !debugger_enabled() {
        return Err(TaskError::bad_request(
            "property flowable.experimental.debugger.enabled is not enabled",
        ));
    }
    Ok(Json(json!([])))
}

async fn debugger_executions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    if !debugger_enabled() {
        return Err(TaskError::bad_request(
            "property flowable.experimental.debugger.enabled is not enabled",
        ));
    }
    let rows = engine
        .get_runtime_store()
        .db_store()
        .find_all::<Value>("executions")
        .unwrap_or_default();
    let data: Vec<_> = rows
        .into_iter()
        .filter(|r| {
            r.get("processInstanceId")
                .and_then(|v| v.as_str())
                .or_else(|| r.get("process_instance_id").and_then(|v| v.as_str()))
                == Some(process_instance_id.as_str())
        })
        .collect();
    Ok(Json(data))
}

async fn debugger_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    if !debugger_enabled() {
        return Err(TaskError::bad_request(
            "property flowable.experimental.debugger.enabled is not enabled",
        ));
    }
    let vars = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()
        .map_err(TaskError::from_engine)?;
    let data: Vec<_> = vars
        .into_iter()
        .filter(|v| v.execution_id == execution_id)
        .map(|v| {
            json!({
                "name": v.name,
                "type": v.variable_type,
                "value": v.value,
            })
        })
        .collect();
    Ok(Json(data))
}

static DEBUG_BREAKPOINTS: std::sync::Mutex<Vec<Value>> = std::sync::Mutex::new(Vec::new());

// ---- IDM helpers ----

#[derive(Debug, Deserialize)]
struct WorkflowUsersQuery {
    filter: Option<String>,
}

async fn workflow_users(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Query(q): Query<WorkflowUsersQuery>,
) -> Result<impl IntoResponse, TaskError> {
    let users = engine
        .get_identity_service()
        .create_user_query()
        .list()
        .map_err(TaskError::from_engine)?;
    let filter = q.filter.unwrap_or_default().to_lowercase();
    let data: Vec<_> = users
        .iter()
        .filter(|u| {
            filter.is_empty()
                || u.id.to_lowercase().contains(&filter)
                || u.first_name
                    .as_deref()
                    .map(|f| f.to_lowercase().contains(&filter))
                    .unwrap_or(false)
                || u.last_name
                    .as_deref()
                    .map(|l| l.to_lowercase().contains(&filter))
                    .unwrap_or(false)
        })
        .map(UserRepresentation::from)
        .collect();
    Ok(Json(ResultListDataRepresentation::from_page(data, 0, None)))
}

async fn workflow_groups(Extension(engine): Extension<Arc<ProcessEngine>>) -> Result<impl IntoResponse, TaskError> {
    let groups = engine
        .get_identity_service()
        .create_group_query()
        .list()
        .map_err(TaskError::from_engine)?;
    let data: Vec<_> = groups
        .into_iter()
        .map(|g| {
            json!({
                "id": g.id,
                "name": g.name,
                "type": g.group_type,
            })
        })
        .collect();
    Ok(Json(ResultListDataRepresentation::from_page(data, 0, None)))
}

async fn workflow_group(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(group_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let groups = engine
        .get_identity_service()
        .create_group_query()
        .list()
        .map_err(TaskError::from_engine)?;
    let g = groups
        .into_iter()
        .find(|g| g.id == group_id)
        .ok_or_else(|| TaskError::not_found(format!("Group {group_id}")))?;
    Ok(Json(json!({
        "id": g.id,
        "name": g.name,
        "type": g.group_type,
    })))
}

async fn get_user(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(user_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let users = engine
        .get_identity_service()
        .create_user_query()
        .list()
        .map_err(TaskError::from_engine)?;
    let u = users
        .iter()
        .find(|u| u.id == user_id)
        .ok_or_else(|| TaskError::not_found(format!("User {user_id}")))?;
    Ok(Json(UserRepresentation::from(u)))
}

async fn list_app_definitions() -> impl IntoResponse {
    Json(ResultListDataRepresentation::<Value>::from_page(
        vec![],
        0,
        Some(0),
    ))
}

async fn get_app_definition(Path(key): Path<String>) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "message": format!("App definition '{key}' not found") })),
    )
}

// ---------------------------------------------------------------------------
// Helpers / errors
// ---------------------------------------------------------------------------

fn find_task(engine: &ProcessEngine, task_id: &str) -> Result<Task, TaskError> {
    let tasks = engine
        .get_task_service()
        .create_task_query()
        .list()
        .map_err(TaskError::from_engine)?;
    tasks
        .into_iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| TaskError::not_found(format!("Task with id: {task_id} does not exist")))
}

fn parse_date(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|e| e.to_string())
                .and_then(|d| {
                    d.and_hms_opt(0, 0, 0)
                        .ok_or_else(|| "invalid date".into())
                        .map(|n| DateTime::<Utc>::from_naive_utc_and_offset(n, Utc))
                })
        })
        .map_err(|e| format!("Invalid date '{s}': {e}"))
}

#[derive(Debug)]
pub struct TaskError {
    status: StatusCode,
    message: String,
}

impl TaskError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
    fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: msg.into(),
        }
    }
    fn from_engine(err: impl std::fmt::Display) -> Self {
        let s = err.to_string();
        if s.to_lowercase().contains("not found") || s.to_lowercase().contains("does not exist") {
            Self::not_found(s)
        } else {
            Self::bad_request(s)
        }
    }
}

impl IntoResponse for TaskError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "message": self.message }))).into_response()
    }
}
