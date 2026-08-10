//! Admin UI app (`/admin-app/**`) — stream B.
//!
//! Transparent HTTP proxy over engine REST, plus ServerConfig CRUD.

mod crypto;
mod display_json;
mod proxy;
mod server_config;

use axum::{
    body::Bytes,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::auth::UiAuth;

pub use proxy::{ProxyClient, ProxyError};
pub use server_config::{
    EndpointType, ServerConfig, ServerConfigRepresentation, ServerConfigStore,
};

/// Shared admin state: config store + HTTP proxy client.
#[derive(Clone)]
pub struct AdminState {
    pub configs: Arc<ServerConfigStore>,
    pub proxy: Arc<ProxyClient>,
}

impl AdminState {
    pub fn new() -> Self {
        let configs = Arc::new(ServerConfigStore::with_defaults());
        let proxy = Arc::new(ProxyClient::new());
        Self { configs, proxy }
    }

    pub fn with_store(configs: Arc<ServerConfigStore>) -> Self {
        Self {
            configs,
            proxy: Arc::new(ProxyClient::new()),
        }
    }
}

impl Default for AdminState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the admin router (mounted under `/admin-app` by `ui_router`).
pub fn router() -> Router {
    router_with_state(AdminState::new())
}

pub fn router_with_state(state: AdminState) -> Router {
    Router::new()
        // Health probe (B0)
        .route("/admin-app/rest/health", get(health))
        // Current session user (the admin app resolves the account before
        // loading any server config)
        .route("/admin-app/rest/account", get(account))
        // ServerConfig CRUD
        .route("/admin-app/rest/server-configs", get(list_server_configs))
        .route(
            "/admin-app/rest/server-configs/default/:endpoint_type_code",
            get(get_default_server_config),
        )
        .route("/admin-app/rest/server-configs/:server_id", put(update_server_config))
        // Engine info
        .route(
            "/admin-app/rest/admin/engine-info/:endpoint_type_code",
            get(get_engine_info),
        )
        // ---- PROCESS domain ----
        .route(
            "/admin-app/rest/admin/deployments",
            get(list_deployments).post(upload_deployment),
        )
        .route(
            "/admin-app/rest/admin/deployments/:deployment_id",
            get(get_deployment).delete(delete_deployment),
        )
        .route(
            "/admin-app/rest/admin/process-definitions",
            get(list_process_definitions),
        )
        .route(
            "/admin-app/rest/admin/process-definitions/:definition_id",
            get(get_process_definition).put(update_process_definition),
        )
        .route(
            "/admin-app/rest/admin/process-definitions/:definition_id/process-instances",
            get(process_definition_instances),
        )
        .route(
            "/admin-app/rest/admin/process-definitions/:definition_id/jobs",
            get(process_definition_jobs),
        )
        .route(
            "/admin-app/rest/admin/process-definitions/:definition_id/batch-migrate",
            post(process_definition_batch_migrate),
        )
        .route(
            "/admin-app/rest/admin/process-definition-decision-tables/:definition_id",
            get(process_definition_decision_tables),
        )
        .route(
            "/admin-app/rest/admin/process-definition-form-definitions/:definition_id",
            get(process_definition_form_definitions),
        )
        .route(
            "/admin-app/rest/admin/process-instances",
            post(list_process_instances),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id",
            get(get_process_instance).post(process_instance_action),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/tasks",
            get(process_instance_tasks),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/variables",
            get(process_instance_variables).post(create_process_instance_variable),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/variables/:variable_name",
            put(update_process_instance_variable).delete(delete_process_instance_variable),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/jobs",
            get(process_instance_jobs),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/subprocesses",
            get(process_instance_subprocesses),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/change-state",
            post(process_instance_change_state),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/migrate",
            post(process_instance_migrate),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/decision-executions",
            get(process_instance_decision_executions),
        )
        .route(
            "/admin-app/rest/admin/process-instance-content-items/:process_instance_id",
            get(process_instance_content_items),
        )
        .route("/admin-app/rest/admin/tasks", post(list_tasks))
        .route(
            "/admin-app/rest/admin/tasks/:task_id",
            get(get_task)
                .delete(delete_task)
                .post(task_action)
                .put(update_task),
        )
        .route(
            "/admin-app/rest/admin/tasks/:task_id/subtasks",
            get(task_subtasks),
        )
        .route(
            "/admin-app/rest/admin/tasks/:task_id/variables",
            get(task_variables),
        )
        .route(
            "/admin-app/rest/admin/tasks/:task_id/identitylinks",
            get(task_identity_links),
        )
        .route("/admin-app/rest/admin/jobs", get(list_jobs))
        .route(
            "/admin-app/rest/admin/jobs/:job_id",
            get(get_job).delete(delete_job).post(execute_job),
        )
        // Java `JobClientResource` path; the legacy admin frontend calls this one
        // (`ui/legacy/admin/admin/scripts/job-controllers.js:98`).
        .route(
            "/admin-app/rest/admin/jobs/:job_id/stacktrace",
            get(job_stacktrace),
        )
        // deprecated alias (engine-REST-style path kept for one version)
        .route(
            "/admin-app/rest/admin/jobs/:job_id/exception-stacktrace",
            get(job_stacktrace),
        )
        .route(
            "/admin-app/rest/admin/move-jobs/:job_id",
            post(move_job),
        )
        .route(
            "/admin-app/rest/admin/event-subscriptions",
            get(list_event_subscriptions),
        )
        .route(
            "/admin-app/rest/admin/event-subscriptions/:event_subscription_id",
            get(get_event_subscription).post(event_subscription_action),
        )
        .route("/admin-app/rest/admin/batches", get(list_batches))
        .route(
            "/admin-app/rest/admin/batches/:batch_id",
            get(get_batch).delete(delete_batch),
        )
        .route(
            "/admin-app/rest/admin/batches/:batch_id/batch-parts",
            get(batch_parts),
        )
        .route(
            "/admin-app/rest/admin/batches/:batch_id/batch-document",
            get(batch_document),
        )
        .route(
            "/admin-app/rest/admin/batch-parts/:batch_part_id",
            get(get_batch_part),
        )
        .route(
            "/admin-app/rest/admin/batch-parts/:batch_part_id/batch-part-document",
            get(batch_part_document),
        )
        .route("/admin-app/rest/admin/models", get(list_models))
        // ---- CMMN domain ----
        .route(
            "/admin-app/rest/admin/cmmn-deployments",
            get(list_cmmn_deployments).post(upload_cmmn_deployment),
        )
        .route(
            "/admin-app/rest/admin/cmmn-deployments/:deployment_id",
            get(get_cmmn_deployment).delete(delete_cmmn_deployment),
        )
        .route(
            "/admin-app/rest/admin/case-definitions",
            get(list_case_definitions),
        )
        .route(
            "/admin-app/rest/admin/case-definitions/:definition_id",
            get(get_case_definition),
        )
        .route(
            "/admin-app/rest/admin/case-definitions/:definition_id/case-instances",
            get(case_definition_instances),
        )
        .route(
            "/admin-app/rest/admin/case-definitions/:definition_id/jobs",
            get(case_definition_jobs),
        )
        .route(
            "/admin-app/rest/admin/case-definitions/:definition_id/model-json",
            get(case_definition_model_json),
        )
        .route(
            "/admin-app/rest/admin/case-definition-decision-tables/:definition_id",
            get(case_definition_decision_tables),
        )
        .route(
            "/admin-app/rest/admin/case-definition-form-definitions/:definition_id",
            get(case_definition_form_definitions),
        )
        .route("/admin-app/rest/admin/case-instances", post(list_case_instances))
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id",
            get(get_case_instance).post(case_instance_action),
        )
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id/tasks",
            get(case_instance_tasks),
        )
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id/variables",
            get(case_instance_variables).post(create_case_instance_variable),
        )
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id/variables/:variable_name",
            put(update_case_instance_variable).delete(delete_case_instance_variable),
        )
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id/jobs",
            get(case_instance_jobs),
        )
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id/change-state",
            post(case_instance_change_state),
        )
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id/migrate",
            post(case_instance_migrate),
        )
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id/decision-executions",
            get(case_instance_decision_executions),
        )
        .route(
            "/admin-app/rest/admin/case-instances/:case_instance_id/model-json",
            get(case_instance_model_json),
        )
        .route("/admin-app/rest/admin/cmmn-tasks", post(list_cmmn_tasks))
        .route(
            "/admin-app/rest/admin/cmmn-tasks/:task_id",
            get(get_cmmn_task)
                .delete(delete_cmmn_task)
                .post(cmmn_task_action)
                .put(update_cmmn_task),
        )
        .route(
            "/admin-app/rest/admin/cmmn-tasks/:task_id/subtasks",
            get(cmmn_task_subtasks),
        )
        .route(
            "/admin-app/rest/admin/cmmn-tasks/:task_id/variables",
            get(cmmn_task_variables),
        )
        .route(
            "/admin-app/rest/admin/cmmn-tasks/:task_id/identitylinks",
            get(cmmn_task_identity_links),
        )
        .route("/admin-app/rest/admin/cmmn-jobs", get(list_cmmn_jobs))
        .route(
            "/admin-app/rest/admin/cmmn-jobs/:job_id",
            get(get_cmmn_job).delete(delete_cmmn_job).post(execute_cmmn_job),
        )
        .route(
            "/admin-app/rest/admin/cmmn-jobs/:job_id/stacktrace",
            get(cmmn_job_stacktrace),
        )
        .route(
            "/admin-app/rest/admin/move-cmmn-jobs/:job_id",
            post(move_cmmn_job),
        )
        // ---- DMN domain ----
        .route(
            "/admin-app/rest/admin/decision-table-deployments",
            get(list_decision_deployments).post(upload_dmn_deployment),
        )
        .route(
            "/admin-app/rest/admin/decision-table-deployments/:deployment_id",
            get(get_decision_deployment).delete(delete_decision_deployment),
        )
        .route(
            "/admin-app/rest/admin/decision-tables",
            get(list_decision_tables),
        )
        .route(
            "/admin-app/rest/admin/decision-tables/:decision_table_id",
            get(get_decision_table),
        )
        .route(
            "/admin-app/rest/admin/decision-tables/:decision_table_id/editorJson",
            get(decision_table_editor_json),
        )
        .route(
            "/admin-app/rest/admin/decision-tables/history",
            get(decision_historic_executions),
        )
        .route(
            "/admin-app/rest/admin/decision-tables/history/:execution_id",
            get(decision_historic_execution),
        )
        .route(
            "/admin-app/rest/admin/decision-tables/history/:execution_id/auditdata",
            get(decision_historic_audit),
        )
        // ---- FORM domain ----
        .route(
            "/admin-app/rest/admin/form-deployments",
            get(list_form_deployments).post(upload_form_deployment),
        )
        .route(
            "/admin-app/rest/admin/form-deployments/:deployment_id",
            get(get_form_deployment).delete(delete_form_deployment),
        )
        .route(
            "/admin-app/rest/admin/form-definitions",
            get(list_form_definitions),
        )
        .route(
            "/admin-app/rest/admin/form-definitions/:form_definition_id",
            get(get_form_definition),
        )
        .route(
            "/admin-app/rest/admin/form-instances",
            get(list_form_instances),
        )
        .route(
            "/admin-app/rest/admin/form-instances/:form_instance_id",
            get(get_form_instance),
        )
        .route(
            "/admin-app/rest/admin/form-instances/:form_instance_id/form-field-values",
            get(form_instance_field_values),
        )
        .route(
            "/admin-app/rest/admin/task-form-instance/:task_id",
            get(task_form_instance),
        )
        .route(
            "/admin-app/rest/admin/form-definition-form-instances/:form_definition_id",
            get(form_definition_form_instances),
        )
        .route(
            "/admin-app/rest/admin/process-form-instances/:process_instance_id",
            get(process_form_instances),
        )
        .route(
            "/admin-app/rest/admin/case-form-instances/:case_instance_id",
            get(case_form_instances),
        )
        // ---- APP domain ----
        .route(
            "/admin-app/rest/admin/app-deployments",
            get(list_app_deployments).post(upload_app_deployment),
        )
        .route(
            "/admin-app/rest/admin/app-deployments/:deployment_id",
            get(get_app_deployment).delete(delete_app_deployment),
        )
        .route(
            "/admin-app/rest/admin/app-definitions",
            get(list_app_definitions),
        )
        .route(
            "/admin-app/rest/admin/app-definitions/:definition_id",
            get(get_app_definition),
        )
        .route(
            "/admin-app/rest/admin/app-definitions/:definition_id/process-definitions",
            get(app_definition_process_definitions),
        )
        .route(
            "/admin-app/rest/admin/app-definitions/:definition_id/case-definitions",
            get(app_definition_case_definitions),
        )
        .route(
            "/admin-app/rest/admin/app-definitions/:definition_id/decision-tables",
            get(app_definition_decision_tables),
        )
        .route(
            "/admin-app/rest/admin/app-definitions/:definition_id/form-definitions",
            get(app_definition_form_definitions),
        )
        // ---- CONTENT domain ----
        .route("/admin-app/rest/admin/content-items", get(list_content_items))
        .route(
            "/admin-app/rest/admin/content-items/:content_item_id",
            get(get_content_item),
        )
        // Display JSON (assembled from BpmnModel DI)
        .route(
            "/admin-app/rest/admin/process-definitions/:process_definition_id/model-json",
            get(process_definition_model_json),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/model-json",
            get(process_instance_model_json),
        )
        .route(
            "/admin-app/rest/admin/process-instances/:process_instance_id/history-model-json",
            get(process_instance_history_model_json),
        )
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "app": "admin" }))
}

/// `GET /admin-app/rest/account` — Java flowable-ui-admin
/// `AccountResource.getAccount`.
///
/// The admin app resolves the current user here on startup and only loads the
/// server configs on success, so without this route the whole admin UI stays
/// inert in enforced mode. Same shape as the task app's account: user fields
/// plus group memberships and effective privilege names.
async fn account(
    auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<Value>, AdminError> {
    let identity = engine.get_identity_service();
    let user = identity
        .find_user_by_id(auth.user_id())
        .ok_or_else(|| AdminError::not_found("Account not found".to_string()))?;
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

// ---------------------------------------------------------------------------
// ServerConfig
// ---------------------------------------------------------------------------

async fn list_server_configs(State(state): State<AdminState>) -> impl IntoResponse {
    Json(state.configs.list_representations())
}

async fn get_default_server_config(
    Path(endpoint_type_code): Path<i32>,
) -> Result<impl IntoResponse, AdminError> {
    let endpoint = EndpointType::from_code(endpoint_type_code)
        .ok_or_else(|| AdminError::bad_request(format!("Unknown endpoint type code: {endpoint_type_code}")))?;
    Ok(Json(ServerConfigStore::default_representation(endpoint)))
}

async fn update_server_config(
    State(state): State<AdminState>,
    Path(server_id): Path<String>,
    Json(body): Json<ServerConfigRepresentation>,
) -> Result<impl IntoResponse, AdminError> {
    state
        .configs
        .update(&server_id, body)
        .map_err(AdminError::bad_request)?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Engine info
// ---------------------------------------------------------------------------

async fn get_engine_info(
    State(state): State<AdminState>,
    Path(endpoint_type_code): Path<i32>,
) -> Result<Response, AdminError> {
    let endpoint = EndpointType::from_code(endpoint_type_code)
        .ok_or_else(|| AdminError::bad_request(format!("No valid endpoint type code provided: {endpoint_type_code}")))?;
    let path = match endpoint {
        EndpointType::Process => "management/engine",
        EndpointType::Dmn => "dmn-management/engine",
        EndpointType::Form => "form-management/engine",
        EndpointType::Content => "content-management/engine",
        EndpointType::Cmmn => "cmmn-management/engine",
        EndpointType::App => "app-management/engine",
    };
    proxy_get(&state, endpoint, path, &[]).await
}

// ---------------------------------------------------------------------------
// PROCESS — deployments / definitions / instances / tasks / jobs
// ---------------------------------------------------------------------------

async fn list_deployments(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::Process, "repository/deployments", &q).await
}

async fn get_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("repository/deployments/{deployment_id}"),
        &[],
    )
    .await
}

async fn delete_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::Process,
        Method::DELETE,
        &format!("repository/deployments/{deployment_id}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn list_process_definitions(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::Process,
        "repository/process-definitions",
        &q,
    )
    .await
}

async fn get_process_definition(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("repository/process-definitions/{definition_id}"),
        &[],
    )
    .await
}

async fn update_process_definition(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::PUT,
        &format!("repository/process-definitions/{definition_id}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn process_definition_instances(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("processDefinitionId".into(), definition_id));
    // Admin lists historic instances for a definition via query API.
    proxy_get(
        &state,
        EndpointType::Process,
        "history/historic-process-instances",
        &q,
    )
    .await
}

async fn process_definition_jobs(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("processDefinitionId".into(), definition_id));
    proxy_get(&state, EndpointType::Process, "management/jobs", &q).await
}

async fn list_process_instances(
    State(state): State<AdminState>,
    body: Bytes,
) -> Result<Response, AdminError> {
    // Java: POST query/historic-process-instances with body + paging query params extracted from body.
    let (uri_extra, body) = extract_paging_from_json_body(body, "query/historic-process-instances")?;
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &uri_extra,
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn get_process_instance(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("history/historic-process-instances/{process_instance_id}"),
        &[],
    )
    .await
}

async fn process_instance_action(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    // Java uses runtime delete / suspend via POST body action.
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("runtime/process-instances/{process_instance_id}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn process_instance_tasks(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("processInstanceId".into(), process_instance_id),
        ("size".into(), "1024".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Process,
        "history/historic-task-instances",
        &q,
    )
    .await
}

async fn process_instance_variables(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("processInstanceId".into(), process_instance_id),
        ("size".into(), "1024".into()),
        ("sort".into(), "variableName".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Process,
        "history/historic-variable-instances",
        &q,
    )
    .await
}

async fn create_process_instance_variable(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("runtime/process-instances/{process_instance_id}/variables"),
        body,
        "application/json",
        StatusCode::CREATED,
    )
    .await
}

async fn update_process_instance_variable(
    State(state): State<AdminState>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::PUT,
        &format!("runtime/process-instances/{process_instance_id}/variables/{variable_name}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn delete_process_instance_variable(
    State(state): State<AdminState>,
    Path((process_instance_id, variable_name)): Path<(String, String)>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::Process,
        Method::DELETE,
        &format!("runtime/process-instances/{process_instance_id}/variables/{variable_name}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn process_instance_jobs(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("processInstanceId".into(), process_instance_id));
    proxy_get(&state, EndpointType::Process, "management/jobs", &q).await
}

async fn process_instance_subprocesses(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("superProcessInstanceId".into(), process_instance_id),
        ("size".into(), "100".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Process,
        "history/historic-process-instances",
        &q,
    )
    .await
}

async fn process_instance_change_state(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("runtime/process-instances/{process_instance_id}/change-state"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn process_instance_migrate(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("runtime/process-instances/{process_instance_id}/migrate"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

/// `POST /rest/admin/process-definitions/{id}/batch-migrate` (Java
/// `ProcessDefinitionClientResource`): migration document forwarded verbatim.
async fn process_definition_batch_migrate(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("repository/process-definitions/{definition_id}/batch-migrate"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

/// `GET /rest/admin/process-definition-decision-tables/{pdId}` (Java
/// `DecisionTablesClientResource`): decision tables referenced by the process.
async fn process_definition_decision_tables(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("repository/process-definitions/{definition_id}/decision-tables"),
        &[],
    )
    .await
}

/// `GET /rest/admin/process-definition-form-definitions/{pdId}` (Java
/// `FormDefinitionsClientResource`).
async fn process_definition_form_definitions(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("repository/process-definitions/{definition_id}/form-definitions"),
        &[],
    )
    .await
}

/// `GET /rest/admin/process-instances/{id}/decision-executions` (Java
/// `ProcessInstanceClientResource`): historic decision executions, DMN endpoint.
async fn process_instance_decision_executions(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("instanceId".into(), process_instance_id),
        ("size".into(), "1024".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Dmn,
        "dmn-history/historic-decision-executions",
        &q,
    )
    .await
}

/// `GET /rest/admin/process-instance-content-items/{processInstanceId}` (Java
/// `ContentItemsClientResource`), CONTENT endpoint.
async fn process_instance_content_items(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("processInstanceId".into(), process_instance_id));
    proxy_get(&state, EndpointType::Content, "content-service/content-items", &q).await
}

async fn list_tasks(
    State(state): State<AdminState>,
    body: Bytes,
) -> Result<Response, AdminError> {
    let (uri_extra, body) = extract_paging_from_json_body(body, "query/historic-task-instances")?;
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &uri_extra,
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn get_task(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let runtime = params
        .get("runtime")
        .map(|v| v == "true")
        .unwrap_or(false);
    let path = if runtime {
        format!("runtime/tasks/{task_id}")
    } else {
        format!("history/historic-task-instances/{task_id}")
    };
    proxy_get(&state, EndpointType::Process, &path, &[]).await
}

async fn delete_task(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    // Prefer runtime cascade; engine returns 404 if already historic-only.
    proxy_no_body(
        &state,
        EndpointType::Process,
        Method::DELETE,
        &format!("runtime/tasks/{task_id}?cascadeHistory=true"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn task_action(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("runtime/tasks/{task_id}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn update_task(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::PUT,
        &format!("runtime/tasks/{task_id}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn task_subtasks(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("parentTaskId".into(), task_id),
        ("size".into(), "1024".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Process,
        "history/historic-task-instances",
        &q,
    )
    .await
}

async fn task_variables(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("taskId".into(), task_id),
        ("size".into(), "1024".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Process,
        "history/historic-variable-instances",
        &q,
    )
    .await
}

async fn task_identity_links(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("history/historic-task-instances/{task_id}/identitylinks"),
        &[],
    )
    .await
}

async fn list_jobs(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    let job_url = job_collection_path(&params);
    proxy_get(&state, EndpointType::Process, job_url, &q).await
}

async fn get_job(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let base = job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("{base}/{job_id}"),
        &[],
    )
    .await
}

async fn delete_job(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let base = job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_no_body(
        &state,
        EndpointType::Process,
        Method::DELETE,
        &format!("{base}/{job_id}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn execute_job(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    body: Bytes,
) -> Result<Response, AdminError> {
    let base = job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("{base}/{job_id}"),
        body,
        "application/json",
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn job_stacktrace(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let base = job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("{base}/{job_id}/exception-stacktrace"),
        &[],
    )
    .await
}

/// `POST /rest/admin/move-jobs/{jobId}` (Java `JobClientResource.moveJob`):
/// moves a timer/suspended/deadletter job back to executable via a
/// server-constructed `{"action":"move"}` body; expects 204 from the engine.
async fn move_job(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let base = job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("{base}/{job_id}"),
        Bytes::from_static(br#"{"action":"move"}"#),
        "application/json",
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn list_event_subscriptions(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::Process,
        "runtime/event-subscriptions",
        &q,
    )
    .await
}

async fn get_event_subscription(
    State(state): State<AdminState>,
    Path(event_subscription_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("runtime/event-subscriptions/{event_subscription_id}"),
        &[],
    )
    .await
}

async fn event_subscription_action(
    State(state): State<AdminState>,
    Path(event_subscription_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Process,
        Method::POST,
        &format!("runtime/event-subscriptions/{event_subscription_id}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn list_batches(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::Process, "management/batches", &q).await
}

async fn get_batch(
    State(state): State<AdminState>,
    Path(batch_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("management/batches/{batch_id}"),
        &[],
    )
    .await
}

async fn delete_batch(
    State(state): State<AdminState>,
    Path(batch_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::Process,
        Method::DELETE,
        &format!("management/batches/{batch_id}"),
        StatusCode::OK,
    )
    .await
}

async fn batch_parts(
    State(state): State<AdminState>,
    Path(batch_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("management/batches/{batch_id}/batch-parts"),
        &q,
    )
    .await
}

async fn batch_document(
    State(state): State<AdminState>,
    Path(batch_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("management/batches/{batch_id}/batch-document"),
        &[],
    )
    .await
}

async fn get_batch_part(
    State(state): State<AdminState>,
    Path(batch_part_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("management/batch-parts/{batch_part_id}"),
        &[],
    )
    .await
}

async fn batch_part_document(
    State(state): State<AdminState>,
    Path(batch_part_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Process,
        &format!("management/batch-parts/{batch_part_id}/batch-part-document"),
        &[],
    )
    .await
}

async fn list_models(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::Process, "repository/models", &q).await
}

// ---------------------------------------------------------------------------
// CMMN
// ---------------------------------------------------------------------------

async fn list_cmmn_deployments(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::Cmmn, "cmmn-repository/deployments", &q).await
}

async fn get_cmmn_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Cmmn,
        &format!("cmmn-repository/deployments/{deployment_id}"),
        &[],
    )
    .await
}

async fn delete_cmmn_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::Cmmn,
        Method::DELETE,
        &format!("cmmn-repository/deployments/{deployment_id}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn list_case_definitions(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::Cmmn,
        "cmmn-repository/case-definitions",
        &q,
    )
    .await
}

async fn get_case_definition(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Cmmn,
        &format!("cmmn-repository/case-definitions/{definition_id}"),
        &[],
    )
    .await
}

async fn case_definition_instances(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("caseDefinitionId".into(), definition_id));
    proxy_get(
        &state,
        EndpointType::Cmmn,
        "cmmn-history/historic-case-instances",
        &q,
    )
    .await
}

async fn case_definition_jobs(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("caseDefinitionId".into(), definition_id));
    proxy_get(&state, EndpointType::Cmmn, "cmmn-management/jobs", &q).await
}

async fn list_case_instances(
    State(state): State<AdminState>,
    body: Bytes,
) -> Result<Response, AdminError> {
    let (uri_extra, body) =
        extract_paging_from_json_body(body, "cmmn-query/historic-case-instances")?;
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &uri_extra,
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn get_case_instance(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Cmmn,
        &format!("cmmn-history/historic-case-instances/{case_instance_id}"),
        &[],
    )
    .await
}

async fn case_instance_action(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &format!("cmmn-runtime/case-instances/{case_instance_id}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn case_instance_tasks(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("caseInstanceId".into(), case_instance_id),
        ("size".into(), "1024".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Cmmn,
        "cmmn-history/historic-task-instances",
        &q,
    )
    .await
}

async fn case_instance_variables(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("caseInstanceId".into(), case_instance_id),
        ("size".into(), "1024".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Cmmn,
        "cmmn-history/historic-variable-instances",
        &q,
    )
    .await
}

async fn create_case_instance_variable(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &format!("cmmn-runtime/case-instances/{case_instance_id}/variables"),
        body,
        "application/json",
        StatusCode::CREATED,
    )
    .await
}

async fn update_case_instance_variable(
    State(state): State<AdminState>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::PUT,
        &format!("cmmn-runtime/case-instances/{case_instance_id}/variables/{variable_name}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn delete_case_instance_variable(
    State(state): State<AdminState>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::Cmmn,
        Method::DELETE,
        &format!("cmmn-runtime/case-instances/{case_instance_id}/variables/{variable_name}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn case_instance_jobs(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("caseInstanceId".into(), case_instance_id));
    proxy_get(&state, EndpointType::Cmmn, "cmmn-management/jobs", &q).await
}

async fn list_cmmn_tasks(
    State(state): State<AdminState>,
    body: Bytes,
) -> Result<Response, AdminError> {
    let (uri_extra, body) =
        extract_paging_from_json_body(body, "cmmn-query/historic-task-instances")?;
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &uri_extra,
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn get_cmmn_task(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Cmmn,
        &format!("cmmn-history/historic-task-instances/{task_id}"),
        &[],
    )
    .await
}

async fn delete_cmmn_task(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::Cmmn,
        Method::DELETE,
        &format!("cmmn-runtime/tasks/{task_id}?cascadeHistory=true"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn cmmn_task_action(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &format!("cmmn-runtime/tasks/{task_id}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn update_cmmn_task(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::PUT,
        &format!("cmmn-runtime/tasks/{task_id}"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

async fn cmmn_task_subtasks(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("parentTaskId".into(), task_id),
        ("size".into(), "1024".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Cmmn,
        "cmmn-history/historic-task-instances",
        &q,
    )
    .await
}

async fn cmmn_task_variables(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![("taskId".into(), task_id), ("size".into(), "1024".into())];
    proxy_get(
        &state,
        EndpointType::Cmmn,
        "cmmn-history/historic-variable-instances",
        &q,
    )
    .await
}

async fn cmmn_task_identity_links(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Cmmn,
        &format!("cmmn-history/historic-task-instances/{task_id}/identitylinks"),
        &[],
    )
    .await
}

async fn list_cmmn_jobs(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::Cmmn, "cmmn-management/jobs", &q).await
}

/// `GET /rest/admin/cmmn-jobs/{jobId}` (Java `CmmnJobClientResource.getJob`).
async fn get_cmmn_job(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let base = cmmn_job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_get(&state, EndpointType::Cmmn, &format!("{base}/{job_id}"), &[]).await
}

/// `DELETE /rest/admin/cmmn-jobs/{jobId}` (Java `CmmnJobClientResource.deleteJob`).
async fn delete_cmmn_job(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let base = cmmn_job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_no_body(
        &state,
        EndpointType::Cmmn,
        Method::DELETE,
        &format!("{base}/{job_id}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

/// `POST /rest/admin/cmmn-jobs/{jobId}` (Java `CmmnJobClientResource.executeJob`):
/// always targets the executable jobs collection (`jobType` is ignored), with a
/// server-constructed `{"action":"execute"}` body; expects 204.
async fn execute_cmmn_job(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &format!("cmmn-management/jobs/{job_id}"),
        Bytes::from_static(br#"{"action":"execute"}"#),
        "application/json",
        StatusCode::NO_CONTENT,
    )
    .await
}

/// `POST /rest/admin/move-cmmn-jobs/{jobId}` (Java `CmmnJobClientResource.moveJob`).
async fn move_cmmn_job(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let base = cmmn_job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &format!("{base}/{job_id}"),
        Bytes::from_static(br#"{"action":"move"}"#),
        "application/json",
        StatusCode::NO_CONTENT,
    )
    .await
}

/// `GET /rest/admin/cmmn-jobs/{jobId}/stacktrace` (Java
/// `CmmnJobClientResource.getJobStacktrace` → engine `exception-stacktrace`).
async fn cmmn_job_stacktrace(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let base = cmmn_job_collection_path(&params).trim_end_matches('/').to_string();
    proxy_get(
        &state,
        EndpointType::Cmmn,
        &format!("{base}/{job_id}/exception-stacktrace"),
        &[],
    )
    .await
}

/// `POST /rest/admin/case-instances/{id}/change-state` (Java
/// `CaseInstanceClientResource.changeState`): body forwarded verbatim.
async fn case_instance_change_state(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &format!("cmmn-runtime/case-instances/{case_instance_id}/change-state"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

/// `POST /rest/admin/case-instances/{id}/migrate` (Java
/// `CaseInstanceClientResource.migrate`): migration document forwarded verbatim.
async fn case_instance_migrate(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
    body: Bytes,
) -> Result<Response, AdminError> {
    proxy_body(
        &state,
        EndpointType::Cmmn,
        Method::POST,
        &format!("cmmn-runtime/case-instances/{case_instance_id}/migrate"),
        body,
        "application/json",
        StatusCode::OK,
    )
    .await
}

/// `GET /rest/admin/case-instances/{id}/decision-executions` (Java
/// `CaseInstanceClientResource`): historic decision executions scoped to the
/// case instance, DMN endpoint.
async fn case_instance_decision_executions(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    let q = vec![
        ("instanceId".into(), case_instance_id),
        ("scopeType".into(), "cmmn".into()),
        ("size".into(), "1024".into()),
    ];
    proxy_get(
        &state,
        EndpointType::Dmn,
        "dmn-history/historic-decision-executions",
        &q,
    )
    .await
}

/// `GET /rest/admin/case-definition-decision-tables/{cdId}` (Java
/// `DecisionTablesClientResource`), CMMN endpoint.
async fn case_definition_decision_tables(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Cmmn,
        &format!("cmmn-repository/case-definitions/{definition_id}/decision-tables"),
        &[],
    )
    .await
}

/// `GET /rest/admin/case-definition-form-definitions/{cdId}` (Java
/// `FormDefinitionsClientResource`), CMMN endpoint.
async fn case_definition_form_definitions(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Cmmn,
        &format!("cmmn-repository/case-definitions/{definition_id}/form-definitions"),
        &[],
    )
    .await
}

// ---------------------------------------------------------------------------
// DMN / FORM / APP / CONTENT (thin proxies)
// ---------------------------------------------------------------------------

async fn list_decision_deployments(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::Dmn, "dmn-repository/deployments", &q).await
}

async fn get_decision_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Dmn,
        &format!("dmn-repository/deployments/{deployment_id}"),
        &[],
    )
    .await
}

async fn delete_decision_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::Dmn,
        Method::DELETE,
        &format!("dmn-repository/deployments/{deployment_id}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn list_decision_tables(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::Dmn,
        "dmn-repository/decision-tables",
        &q,
    )
    .await
}

async fn get_decision_table(
    State(state): State<AdminState>,
    Path(decision_table_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Dmn,
        &format!("dmn-repository/decision-tables/{decision_table_id}"),
        &[],
    )
    .await
}

async fn decision_table_editor_json(
    State(state): State<AdminState>,
    Path(decision_table_id): Path<String>,
) -> Result<Response, AdminError> {
    // Engine serves model resource; path may vary — use decision resource data.
    proxy_get(
        &state,
        EndpointType::Dmn,
        &format!("dmn-repository/decision-tables/{decision_table_id}/model"),
        &[],
    )
    .await
}

async fn decision_historic_execution(
    State(state): State<AdminState>,
    Path(execution_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Dmn,
        &format!("dmn-history/historic-decision-executions/{execution_id}"),
        &[],
    )
    .await
}

async fn decision_historic_audit(
    State(state): State<AdminState>,
    Path(execution_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Dmn,
        &format!("dmn-history/historic-decision-executions/{execution_id}/auditdata"),
        &[],
    )
    .await
}

/// `GET /rest/admin/decision-tables/history` (Java
/// `DecisionTableHistoricExecutionsClientResource`): list historic decision
/// executions, all query params except `serverId` forwarded.
async fn decision_historic_executions(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::Dmn,
        "dmn-history/historic-decision-executions",
        &q,
    )
    .await
}

async fn list_form_deployments(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::Form, "form-repository/deployments", &q).await
}

async fn get_form_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Form,
        &format!("form-repository/deployments/{deployment_id}"),
        &[],
    )
    .await
}

async fn delete_form_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::Form,
        Method::DELETE,
        &format!("form-repository/deployments/{deployment_id}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn list_form_definitions(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::Form,
        "form-repository/form-definitions",
        &q,
    )
    .await
}

async fn get_form_definition(
    State(state): State<AdminState>,
    Path(form_definition_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Form,
        &format!("form-repository/form-definitions/{form_definition_id}"),
        &[],
    )
    .await
}

async fn list_form_instances(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::Form, "form/form-instances", &q).await
}

async fn get_form_instance(
    State(state): State<AdminState>,
    Path(form_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Form,
        &format!("form/form-instances/{form_instance_id}"),
        &[],
    )
    .await
}

/// `GET /rest/admin/task-form-instance/{taskId}` (Java
/// `FormInstanceClientResource.getTaskFormInstance`): form instance for a task.
async fn task_form_instance(
    State(state): State<AdminState>,
    Path(task_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("taskId".into(), task_id));
    proxy_get(&state, EndpointType::Form, "form/form-instances", &q).await
}

/// `GET /rest/admin/form-instances/{id}/form-field-values` (Java
/// `FormInstanceClientResource.getFormInstanceFieldValues`; engine carries the
/// values on `form/form-instances/{id}/values`).
async fn form_instance_field_values(
    State(state): State<AdminState>,
    Path(form_instance_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Form,
        &format!("form/form-instances/{form_instance_id}/values"),
        &[],
    )
    .await
}

/// `GET /rest/admin/form-definition-form-instances/{fdId}` (Java
/// `FormInstancesClientResource`): form instances by form definition.
async fn form_definition_form_instances(
    State(state): State<AdminState>,
    Path(form_definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("formDefinitionId".into(), form_definition_id));
    proxy_get(&state, EndpointType::Form, "form/form-instances", &q).await
}

/// `GET /rest/admin/process-form-instances/{piId}` (Java
/// `FormInstancesClientResource`): form instances by process instance.
async fn process_form_instances(
    State(state): State<AdminState>,
    Path(process_instance_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("processInstanceId".into(), process_instance_id));
    proxy_get(&state, EndpointType::Form, "form/form-instances", &q).await
}

/// `GET /rest/admin/case-form-instances/{ciId}` (Java
/// `FormInstancesClientResource`): form instances by case instance, expressed
/// as `scopeId` + `scopeType=cmmn` like the Java query body.
async fn case_form_instances(
    State(state): State<AdminState>,
    Path(case_instance_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let mut q = query_without_server_id(&params);
    q.push(("scopeId".into(), case_instance_id));
    q.push(("scopeType".into(), "cmmn".into()));
    proxy_get(&state, EndpointType::Form, "form/form-instances", &q).await
}

async fn list_app_deployments(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(&state, EndpointType::App, "app-repository/deployments", &q).await
}

async fn get_app_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::App,
        &format!("app-repository/deployments/{deployment_id}"),
        &[],
    )
    .await
}

async fn delete_app_deployment(
    State(state): State<AdminState>,
    Path(deployment_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_no_body(
        &state,
        EndpointType::App,
        Method::DELETE,
        &format!("app-repository/deployments/{deployment_id}"),
        StatusCode::NO_CONTENT,
    )
    .await
}

async fn list_app_definitions(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::App,
        "app-repository/app-definitions",
        &q,
    )
    .await
}

async fn get_app_definition(
    State(state): State<AdminState>,
    Path(definition_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::App,
        &format!("app-repository/app-definitions/{definition_id}"),
        &[],
    )
    .await
}

/// Java `AppDefinitionService`: an app deployment spawns child deployments per
/// domain; the related models live in the child deployment, resolved via
/// `?parentDeploymentId=` and then listed with `?deploymentId=`.
async fn app_definition_related(
    state: &AdminState,
    params: &HashMap<String, String>,
    endpoint: EndpointType,
    deployments_path: &str,
    collection_path: &str,
) -> Result<Response, AdminError> {
    let deployment_id = params
        .get("deploymentId")
        .cloned()
        .ok_or_else(|| AdminError::bad_request("Deployment id is required"))?;
    let deployments = proxy_get_json_value(
        state,
        endpoint,
        deployments_path,
        &[("parentDeploymentId".into(), deployment_id)],
    )
    .await?;
    let child_deployment_id = deployments
        .get("data")
        .and_then(Value::as_array)
        .and_then(|data| data.first())
        .and_then(|row| row.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    match child_deployment_id {
        Some(id) => {
            proxy_get(state, endpoint, collection_path, &[("deploymentId".into(), id)]).await
        }
        // Java returns an empty result node when there is no child deployment.
        None => Ok(Json(json!({ "size": 0, "data": [] })).into_response()),
    }
}

async fn app_definition_process_definitions(
    State(state): State<AdminState>,
    Path(_definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    app_definition_related(
        &state,
        &params,
        EndpointType::Process,
        "repository/deployments",
        "repository/process-definitions",
    )
    .await
}

async fn app_definition_case_definitions(
    State(state): State<AdminState>,
    Path(_definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    app_definition_related(
        &state,
        &params,
        EndpointType::Cmmn,
        "cmmn-repository/deployments",
        "cmmn-repository/case-definitions",
    )
    .await
}

async fn app_definition_decision_tables(
    State(state): State<AdminState>,
    Path(_definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    app_definition_related(
        &state,
        &params,
        EndpointType::Dmn,
        "dmn-repository/deployments",
        "dmn-repository/decision-tables",
    )
    .await
}

async fn app_definition_form_definitions(
    State(state): State<AdminState>,
    Path(_definition_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    app_definition_related(
        &state,
        &params,
        EndpointType::Form,
        "form-repository/deployments",
        "form-repository/form-definitions",
    )
    .await
}

async fn list_content_items(
    State(state): State<AdminState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let q = query_without_server_id(&params);
    proxy_get(
        &state,
        EndpointType::Content,
        "content-service/content-items",
        &q,
    )
    .await
}

async fn get_content_item(
    State(state): State<AdminState>,
    Path(content_item_id): Path<String>,
) -> Result<Response, AdminError> {
    proxy_get(
        &state,
        EndpointType::Content,
        &format!("content-service/content-items/{content_item_id}"),
        &[],
    )
    .await
}

async fn upload_deployment(
    State(state): State<AdminState>,
    multipart: axum::extract::Multipart,
) -> Result<Response, AdminError> {
    upload_to_engine(
        &state,
        EndpointType::Process,
        "repository/deployments",
        multipart,
        &[".bpmn", ".bpmn20.xml", ".zip", ".bar"],
    )
    .await
}

async fn upload_cmmn_deployment(
    State(state): State<AdminState>,
    multipart: axum::extract::Multipart,
) -> Result<Response, AdminError> {
    upload_to_engine(
        &state,
        EndpointType::Cmmn,
        "cmmn-repository/deployments",
        multipart,
        &[".cmmn", ".cmmn.xml", ".zip", ".bar"],
    )
    .await
}

async fn upload_dmn_deployment(
    State(state): State<AdminState>,
    multipart: axum::extract::Multipart,
) -> Result<Response, AdminError> {
    upload_to_engine(
        &state,
        EndpointType::Dmn,
        "dmn-repository/deployments",
        multipart,
        &[".dmn", ".dmn.xml", ".zip", ".bar"],
    )
    .await
}

async fn upload_form_deployment(
    State(state): State<AdminState>,
    multipart: axum::extract::Multipart,
) -> Result<Response, AdminError> {
    upload_to_engine(
        &state,
        EndpointType::Form,
        "form-repository/deployments",
        multipart,
        &[".form", ".json", ".zip", ".bar"],
    )
    .await
}

async fn upload_app_deployment(
    State(state): State<AdminState>,
    multipart: axum::extract::Multipart,
) -> Result<Response, AdminError> {
    upload_to_engine(
        &state,
        EndpointType::App,
        "app-repository/deployments",
        multipart,
        &[".zip", ".bar", ".app"],
    )
    .await
}

async fn upload_to_engine(
    state: &AdminState,
    endpoint: EndpointType,
    engine_path: &str,
    mut multipart: axum::extract::Multipart,
    allowed_ext: &[&str],
) -> Result<Response, AdminError> {
    let config = state
        .configs
        .get_by_endpoint(endpoint)
        .map_err(AdminError::bad_request)?;
    let password = state
        .configs
        .decrypt_password(&config)
        .map_err(AdminError::bad_request)?;

    let mut file_name = None;
    let mut file_bytes = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AdminError::bad_request(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || file_bytes.is_none() {
            file_name = field.file_name().map(|s| s.to_string());
            file_bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| AdminError::bad_request(e.to_string()))?,
            );
        }
    }
    let file_name = file_name
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AdminError::bad_request("No file found in POST body"))?;
    let bytes = file_bytes.ok_or_else(|| AdminError::bad_request("No file found in POST body"))?;
    let lower = file_name.to_lowercase();
    if !allowed_ext.iter().any(|ext| lower.ends_with(ext)) {
        return Err(AdminError::bad_request("Invalid file name"));
    }

    let url = crate::admin::proxy::build_server_url(&config, engine_path);
    let part = reqwest::multipart::Part::bytes(bytes.to_vec())
        .file_name(file_name)
        .mime_str("application/octet-stream")
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    let form = reqwest::multipart::Form::new().part("file", part);
    let token = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        format!("{}:{}", config.user_name, password),
    );
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header(
            axum::http::header::AUTHORIZATION.as_str(),
            format!("Basic {token}"),
        )
        .multipart(form)
        .send()
        .await
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    let status = StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let body = response
        .bytes()
        .await
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    if status.is_success() {
        Ok(Response::builder()
            .status(status)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
    } else {
        Err(AdminError::bad_request(format!(
            "Deployment failed with status {status}"
        )))
    }
}

// ---------------------------------------------------------------------------
// Display JSON (in-process BpmnModel when engine Extension is present)
// ---------------------------------------------------------------------------

async fn process_definition_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
) -> Result<impl IntoResponse, AdminError> {
    let model = engine
        .get_repository_service()
        .get_bpmn_model(&process_definition_id)
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    Ok(Json(display_json::build_process_definition_display(
        model.as_ref(),
    )))
}

async fn process_instance_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, AdminError> {
    let pd_id = params
        .get("processDefinitionId")
        .cloned()
        .ok_or_else(|| AdminError::bad_request("processDefinitionId is required"))?;
    let model = engine
        .get_repository_service()
        .get_bpmn_model(&pd_id)
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    let completed = historic_activity_ids(&engine, &process_instance_id, true);
    let current = runtime_activity_ids(&engine, &process_instance_id);
    Ok(Json(display_json::build_process_instance_display(
        model.as_ref(),
        &completed,
        &current,
    )))
}

async fn process_instance_history_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, AdminError> {
    let pd_id = params
        .get("processDefinitionId")
        .cloned()
        .ok_or_else(|| AdminError::bad_request("processDefinitionId is required"))?;
    let model = engine
        .get_repository_service()
        .get_bpmn_model(&pd_id)
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    let completed = historic_activity_ids(&engine, &process_instance_id, false);
    Ok(Json(display_json::build_history_display(
        model.as_ref(),
        &completed,
    )))
}

// ---------------------------------------------------------------------------
// CMMN display JSON (Java `CmmnDisplayJsonClientResource`; assembled in-process
// from the CMMN engine when the Extension is present)
// ---------------------------------------------------------------------------

fn admin_cmmn_engine(
    engine: &ProcessEngine,
) -> Result<Arc<flowable_cmmn_engine::CmmnEngine>, AdminError> {
    engine
        .get_config()
        .cmmn_engine
        .clone()
        .ok_or_else(|| AdminError::bad_request("CMMN engine is not configured on this process engine"))
}

async fn case_definition_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(definition_id): Path<String>,
) -> Result<impl IntoResponse, AdminError> {
    let cmmn = admin_cmmn_engine(&engine)?;
    let definition = cmmn
        .repository_service()
        .get_case_definition(&definition_id)
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    // The Rust CMMN converter does not parse CMMNDI, so no graphic info is
    // available; Java returns an empty display object in that case too.
    Ok(Json(display_json::build_case_definition_display(
        &definition.model,
        &HashMap::new(),
    )))
}

async fn case_instance_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(case_instance_id): Path<String>,
) -> Result<impl IntoResponse, AdminError> {
    let cmmn = admin_cmmn_engine(&engine)?;
    // Java resolves the case definition id from the (historic) case instance.
    let case_definition_id = match cmmn.runtime_service().get_case_instance(&case_instance_id) {
        Ok(instance) => instance.case_definition_id,
        Err(_) => {
            cmmn.history_service()
                .get_historic_case_instance(&case_instance_id)
                .map_err(|e| AdminError::bad_request(e.to_string()))?
                .case_definition_id
        }
    };
    let definition = cmmn
        .repository_service()
        .get_case_definition(&case_definition_id)
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    // Java: plan item instances of the case instance drive the highlighting —
    // completed when completed/terminated/occurred time is set, `active` →
    // current, `available` → available; matched on planItemDefinitionId.
    let plan_item_instances = cmmn
        .runtime_service()
        .create_plan_item_instance_query()
        .case_instance_id(case_instance_id)
        .include_ended()
        .list()
        .unwrap_or_default();
    let mut completed = Vec::new();
    let mut current = Vec::new();
    let mut available = Vec::new();
    for item in &plan_item_instances {
        if item.ended_at.is_some() || item.occurred_at.is_some() {
            completed.push(item.plan_item_definition_id.clone());
        }
        if item.state.eq_ignore_ascii_case("active") {
            current.push(item.plan_item_definition_id.clone());
        }
        if item.state.eq_ignore_ascii_case("available") {
            available.push(item.plan_item_definition_id.clone());
        }
    }
    Ok(Json(display_json::build_case_instance_display(
        &definition.model,
        &HashMap::new(),
        &completed,
        &current,
        &available,
    )))
}

fn historic_activity_ids(engine: &ProcessEngine, process_instance_id: &str, only_finished: bool) -> Vec<String> {
    // Best-effort: read historic activity instances from the store if present.
    let Ok(rows) = engine
        .get_runtime_store()
        .db_store()
        .find_all::<serde_json::Value>("historic_activity_instances")
    else {
        return Vec::new();
    };
    rows.into_iter()
        .filter(|r| {
            r.get("processInstanceId")
                .and_then(|v| v.as_str())
                .or_else(|| r.get("process_instance_id").and_then(|v| v.as_str()))
                == Some(process_instance_id)
        })
        .filter(|r| {
            if !only_finished {
                return true;
            }
            r.get("endTime")
                .or_else(|| r.get("end_time"))
                .map(|v| !v.is_null())
                .unwrap_or(false)
        })
        .filter_map(|r| {
            r.get("activityId")
                .or_else(|| r.get("activity_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect()
}

fn runtime_activity_ids(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let Ok(rows) = engine
        .get_runtime_store()
        .db_store()
        .find_all::<serde_json::Value>("executions")
    else {
        return Vec::new();
    };
    rows.into_iter()
        .filter(|r| {
            r.get("processInstanceId")
                .and_then(|v| v.as_str())
                .or_else(|| r.get("process_instance_id").and_then(|v| v.as_str()))
                == Some(process_instance_id)
        })
        .filter_map(|r| {
            r.get("activityId")
                .or_else(|| r.get("activity_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Proxy helpers
// ---------------------------------------------------------------------------

fn query_without_server_id(params: &HashMap<String, String>) -> Vec<(String, String)> {
    params
        .iter()
        .filter(|(k, _)| k.as_str() != "serverId")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn job_collection_path(params: &HashMap<String, String>) -> &'static str {
    match params.get("jobType").map(|s| s.as_str()) {
        Some("timer") | Some("timerJob") => "management/timer-jobs",
        Some("suspended") | Some("suspendedJob") => "management/suspended-jobs",
        Some("deadletter") | Some("deadletterJob") => "management/deadletter-jobs",
        _ => "management/jobs",
    }
}

/// CMMN counterpart of `job_collection_path` (Java `CmmnJobService.getJobUrl`).
fn cmmn_job_collection_path(params: &HashMap<String, String>) -> &'static str {
    match params.get("jobType").map(|s| s.as_str()) {
        Some("timer") | Some("timerJob") => "cmmn-management/timer-jobs",
        Some("suspended") | Some("suspendedJob") => "cmmn-management/suspended-jobs",
        Some("deadletter") | Some("deadletterJob") => "cmmn-management/deadletter-jobs",
        _ => "cmmn-management/jobs",
    }
}

/// Pull size/sort/order out of a JSON body into the URI query (Java FlowableClientService).
fn extract_paging_from_json_body(
    body: Bytes,
    base_path: &str,
) -> Result<(String, Bytes), AdminError> {
    if body.is_empty() {
        return Ok((base_path.to_string(), body));
    }
    let mut value: Value = serde_json::from_slice(&body)
        .map_err(|e| AdminError::bad_request(format!("Invalid JSON body: {e}")))?;
    let mut pairs = Vec::new();
    if let Some(obj) = value.as_object_mut() {
        for key in ["size", "sort", "order"] {
            if let Some(v) = obj.remove(key) {
                let text = match v {
                    Value::String(s) => s,
                    other => other.to_string().trim_matches('"').to_string(),
                };
                pairs.push(format!("{key}={text}"));
            }
        }
    }
    let uri = if pairs.is_empty() {
        base_path.to_string()
    } else {
        format!("{base_path}?{}", pairs.join("&"))
    };
    let new_body = Bytes::from(
        serde_json::to_vec(&value).map_err(|e| AdminError::bad_request(e.to_string()))?,
    );
    Ok((uri, new_body))
}

async fn proxy_get(
    state: &AdminState,
    endpoint: EndpointType,
    path: &str,
    query: &[(String, String)],
) -> Result<Response, AdminError> {
    let config = state
        .configs
        .get_by_endpoint(endpoint)
        .map_err(AdminError::bad_request)?;
    let password = state
        .configs
        .decrypt_password(&config)
        .map_err(AdminError::bad_request)?;
    state
        .proxy
        .execute_json(
            &config,
            &password,
            Method::GET,
            path,
            query,
            None,
            None,
            StatusCode::OK,
        )
        .await
        .map_err(AdminError::from)
}

/// `proxy_get` variant that parses the upstream JSON body (two-step lookups
/// such as the app-definition related-model resolution).
async fn proxy_get_json_value(
    state: &AdminState,
    endpoint: EndpointType,
    path: &str,
    query: &[(String, String)],
) -> Result<Value, AdminError> {
    let response = proxy_get(state, endpoint, path, query).await?;
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| AdminError::bad_request(e.to_string()))?;
    Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn proxy_body(
    state: &AdminState,
    endpoint: EndpointType,
    method: Method,
    path: &str,
    body: Bytes,
    content_type: &str,
    expected: StatusCode,
) -> Result<Response, AdminError> {
    let config = state
        .configs
        .get_by_endpoint(endpoint)
        .map_err(AdminError::bad_request)?;
    let password = state
        .configs
        .decrypt_password(&config)
        .map_err(AdminError::bad_request)?;
    state
        .proxy
        .execute_json(
            &config,
            &password,
            method,
            path,
            &[],
            Some(body),
            Some(content_type),
            expected,
        )
        .await
        .map_err(AdminError::from)
}

async fn proxy_no_body(
    state: &AdminState,
    endpoint: EndpointType,
    method: Method,
    path: &str,
    expected: StatusCode,
) -> Result<Response, AdminError> {
    let config = state
        .configs
        .get_by_endpoint(endpoint)
        .map_err(AdminError::bad_request)?;
    let password = state
        .configs
        .decrypt_password(&config)
        .map_err(AdminError::bad_request)?;
    state
        .proxy
        .execute_json(
            &config,
            &password,
            method,
            path,
            &[],
            None,
            None,
            expected,
        )
        .await
        .map_err(AdminError::from)
}

// ---------------------------------------------------------------------------
// Errors (align with Java BadRequestException wrapping for admin UI)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct AdminError {
    status: StatusCode,
    message: String,
}

impl AdminError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
}

impl From<ProxyError> for AdminError {
    fn from(value: ProxyError) -> Self {
        // Java wraps most proxy failures as BadRequestException with a message.
        Self::bad_request(value.to_string())
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "message": self.message }));
        (self.status, body).into_response()
    }
}

// Silence unused import warnings for HeaderMap/Deserialize in this module.
#[allow(dead_code)]
fn _markers() {
    let _: Option<HeaderMap> = None;
    #[derive(Deserialize)]
    struct _Q {
        #[allow(dead_code)]
        x: Option<String>,
    }
}
