//! Task-variable routes rebuilt to the Java REST contract.
//!
//! Java parity: mirrors `TaskVariableCollectionResource`,
//! `TaskVariableResource`, `TaskVariableDataResource` and the shared
//! `TaskVariableBaseResource` logic from the Java Flowable REST API.
//!
//! Scope resolution: a per-variable `scope` field in the JSON body (Java
//! standard) wins over the `?scope=` query parameter (kept as a Rust
//! extension); when neither is present writes default to the local scope,
//! matching Java. Reads without a scope return the local value first and fall
//! back to the execution (global) scope.

use crate::common::parse_query;
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{FromRequest, Multipart, Path, Request},
    http::{StatusCode, Uri, header},
    response::Response,
    routing::get,
};
use flowable_engine::cmd::task_variable_cmd::TaskVariableScope;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::task::Task;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use super::process_instances::{
    RestVariableResponse, VariableRequest, encode_variable_data, parse_variable_requests,
    storage_value_for_data_backed_variable_request, to_rest_variable_response,
    variable_data_response, variable_data_type, variables_for_execution,
};

const TASK_VARIABLES_PATH: &str = "/runtime/tasks/:id/variables";
const TASK_VARIABLE_PATH: &str = "/runtime/tasks/:id/variables/:variable_name";
const TASK_VARIABLE_DATA_PATH: &str = "/runtime/tasks/:id/variables/:variable_name/data";

// --- P142c resource limits -------------------------------------------------
// axum `DefaultBodyLimit` does not apply to Multipart; raw JSON used
// `to_bytes(..., usize::MAX)`. Fixed consts (not config) — P142a owns config.

/// Single file part / JSON body cap (64 MiB).
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_MULTIPART_FILE_BYTES: usize = MAX_REQUEST_BODY_BYTES;
/// Cumulative bytes across all parts of one multipart request (256 MiB).
const MAX_MULTIPART_REQUEST_BYTES: usize = 256 * 1024 * 1024;
/// Text form fields (name/type/scope) stay small; still stream-counted.
const MAX_MULTIPART_TEXT_FIELD_BYTES: usize = 1024 * 1024;

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{TASK_VARIABLES_PATH}"),
            get(list_task_variables)
                .post(create_task_variables)
                .put(update_task_variables)
                .delete(delete_all_local_task_variables),
        )
        .route(
            &format!("{prefix}{TASK_VARIABLE_PATH}"),
            get(get_task_variable)
                .put(update_task_variable)
                .delete(delete_task_variable),
        )
        .route(
            &format!("{prefix}{TASK_VARIABLE_DATA_PATH}"),
            get(get_task_variable_data).put(update_task_variable_data),
        )
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TaskVariableQuery {
    scope: Option<String>,
}

/// Parses the `?scope=` query parameter, kept as a Rust extension next to the
/// Java per-variable body scope. Case-insensitive like Java
/// `RestVariable.getScopeFromString`; the 400 message keeps the pre-existing
/// Rust wording used before the Java-parity rebuild.
fn requested_variable_scope(uri: &Uri) -> Result<Option<TaskVariableScope>, ApiError> {
    let query: TaskVariableQuery = parse_query(uri)?;
    match query.scope.as_deref() {
        None => Ok(None),
        Some(scope) if scope.eq_ignore_ascii_case("local") => Ok(Some(TaskVariableScope::Local)),
        Some(scope) if scope.eq_ignore_ascii_case("global") => Ok(Some(TaskVariableScope::Global)),
        Some(scope) => Err(ApiError::bad_request(format!(
            "Unsupported task variable scope '{scope}'"
        ))),
    }
}

/// Java parity: `RestVariable.getScopeFromString` — invalid scope strings in a
/// request body or multipart form are a 400 with the Java message.
fn body_variable_scope(scope: Option<&str>) -> Result<Option<TaskVariableScope>, ApiError> {
    match scope {
        None => Ok(None),
        Some(scope) if scope.eq_ignore_ascii_case("local") => Ok(Some(TaskVariableScope::Local)),
        Some(scope) if scope.eq_ignore_ascii_case("global") => Ok(Some(TaskVariableScope::Global)),
        Some(scope) => Err(ApiError::bad_request(format!(
            "Invalid variable scope: '{scope}'"
        ))),
    }
}

/// Resolves the effective write scope: the per-variable body scope wins over
/// the `?scope=` query fallback; with neither present Java defaults to local.
fn resolve_write_scope(
    body_scope: Option<&str>,
    query_scope: Option<TaskVariableScope>,
) -> Result<TaskVariableScope, ApiError> {
    Ok(body_variable_scope(body_scope)?
        .or(query_scope)
        .unwrap_or(TaskVariableScope::Local))
}

fn scope_label(scope: TaskVariableScope) -> &'static str {
    match scope {
        TaskVariableScope::Local => "local",
        TaskVariableScope::Global => "global",
    }
}

fn scoped_variable_response(
    name: String,
    value: Value,
    scope: TaskVariableScope,
) -> RestVariableResponse {
    let mut response = to_rest_variable_response(name, value);
    response.scope = scope_label(scope).to_string();
    response
}

/// Java 404 message shared by the single-variable reads
/// (`TaskVariableBaseResource.getVariableFromRequestWithoutAccessCheck`).
fn variable_not_found(task_id: &str, variable_name: &str) -> ApiError {
    ApiError::NotFound(format!(
        "Task '{task_id}' does not have a variable with name: '{variable_name}'."
    ))
}

/// Java `getVariableFromRequestWithoutAccessCheck`: without a scope the local
/// value wins and the execution (global) scope is the fallback; a standalone
/// task has no global scope at all.
fn find_task_variable_value(
    engine: &ProcessEngine,
    task: &Task,
    variable_name: &str,
    scope: Option<TaskVariableScope>,
) -> Result<(Value, TaskVariableScope), ApiError> {
    if scope != Some(TaskVariableScope::Global)
        && let Some(value) = engine
            .get_task_service()
            .get_task_local_variable(task.id.clone(), variable_name.to_string())?
    {
        return Ok((value, TaskVariableScope::Local));
    }
    if scope != Some(TaskVariableScope::Local)
        && !task.execution_id.is_empty()
        && let Some(value) = engine
            .get_variable_service()
            .get_variable(task.execution_id.clone(), variable_name.to_string())?
    {
        return Ok((value, TaskVariableScope::Global));
    }
    Err(variable_not_found(&task.id, variable_name))
}

async fn list_task_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    uri: Uri,
) -> Result<Json<Vec<RestVariableResponse>>, ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    let scope = requested_variable_scope(&uri)?;
    // Java getVariables: local variables go into the map first; globals only
    // fill names that are not already present (local precedence).
    let mut variables = Vec::new();
    if scope != Some(TaskVariableScope::Global) {
        let mut locals = engine
            .get_task_service()
            .get_task_local_variables(task.id.clone())?
            .into_iter()
            .collect::<Vec<_>>();
        locals.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, value) in locals {
            variables.push(scoped_variable_response(
                name,
                value,
                TaskVariableScope::Local,
            ));
        }
    }
    if scope != Some(TaskVariableScope::Local) && !task.execution_id.is_empty() {
        for mut variable in variables_for_execution(&engine, &task.execution_id)? {
            if variables
                .iter()
                .any(|existing: &RestVariableResponse| existing.name == variable.name)
            {
                continue;
            }
            variable.scope = "global".to_string();
            variables.push(variable);
        }
    }
    Ok(Json(variables))
}

async fn get_task_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, variable_name)): Path<(String, String)>,
    uri: Uri,
) -> Result<Json<RestVariableResponse>, ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    let (value, scope) = find_task_variable_value(
        &engine,
        &task,
        &variable_name,
        requested_variable_scope(&uri)?,
    )?;
    Ok(Json(scoped_variable_response(variable_name, value, scope)))
}

/// Java POST: create-only on a single shared scope (409 when any variable is
/// already present on that scope, nothing written on failure). Accepts the
/// Java JSON array body, the Rust single-object extension, and
/// multipart/form-data for binary/serializable variables (Java
/// `setBinaryVariable`).
async fn create_task_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    request: Request,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    let query_scope = requested_variable_scope(request.uri())?;
    if is_multipart(&request) {
        let multipart = Multipart::from_request(request, &())
            .await
            .map_err(multipart_error)?;
        let form = parse_binary_variable_form(multipart).await?;
        let (name, value, form_scope) = binary_variable_from_form(form)?;
        let scope = resolve_write_scope(form_scope.as_deref(), query_scope)?;
        engine.get_task_service().create_task_variables(
            task.id.clone(),
            scope,
            HashMap::from([(name.clone(), value.clone())]),
        )?;
        let response = scoped_variable_response(name, value, scope);
        // Java returns the single created RestVariable for multipart creates.
        return Ok((
            StatusCode::CREATED,
            Json(
                serde_json::to_value(response)
                    .map_err(|err| ApiError::InternalServerError(err.to_string()))?,
            ),
        ));
    }
    let body = request_body_string(request).await?;
    let requests = parse_variable_requests(&body)?;
    // Java: an empty variable list is a 400.
    if requests.is_empty() {
        return Err(ApiError::bad_request(
            "Request did not contain a list of variables to create.",
        ));
    }
    let mut shared_scope: Option<TaskVariableScope> = None;
    let mut variables = HashMap::with_capacity(requests.len());
    let mut created_order = Vec::with_capacity(requests.len());
    for request in &requests {
        let name = request
            .name
            .clone()
            .ok_or_else(|| ApiError::BadRequest("Variable name is required".to_string()))?;
        let scope = resolve_write_scope(request.scope.as_deref(), query_scope)?;
        // Java: all variables in one POST must resolve to the same scope.
        match shared_scope {
            None => shared_scope = Some(scope),
            Some(shared) if shared != scope => {
                return Err(ApiError::bad_request(
                    "Only allowed to update multiple variables in the same scope.",
                ));
            }
            _ => {}
        }
        let value = storage_value_for_data_backed_variable_request(request)?;
        variables.insert(name.clone(), value.clone());
        created_order.push((name, value));
    }
    let scope = shared_scope.expect("non-empty create batch has a shared scope");
    engine
        .get_task_service()
        .create_task_variables(task.id.clone(), scope, variables)?;
    // Java returns the created variables in input order.
    let responses = created_order
        .into_iter()
        .map(|(name, value)| scoped_variable_response(name, value, scope))
        .collect::<Vec<_>>();
    Ok((
        StatusCode::CREATED,
        Json(
            serde_json::to_value(responses)
                .map_err(|err| ApiError::InternalServerError(err.to_string()))?,
        ),
    ))
}

/// Rust extension (no Java counterpart): batch upsert. The scope comes from
/// `?scope=` only and still defaults to global so existing extension clients
/// keep their pre-parity behavior.
async fn update_task_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
    uri: Uri,
    body: String,
) -> Result<Json<Vec<RestVariableResponse>>, ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    let scope = requested_variable_scope(&uri)?.unwrap_or(TaskVariableScope::Global);
    let requests = parse_variable_requests(&body)?;
    let mut variables = HashMap::with_capacity(requests.len());
    let mut updated_order = Vec::with_capacity(requests.len());
    for request in &requests {
        let name = request
            .name
            .clone()
            .ok_or_else(|| ApiError::BadRequest("Variable name is required".to_string()))?;
        let value = storage_value_for_data_backed_variable_request(request)?;
        variables.insert(name.clone(), value.clone());
        updated_order.push((name, value));
    }
    match scope {
        TaskVariableScope::Local => engine
            .get_task_service()
            .set_task_variables_local(task.id.clone(), variables)?,
        TaskVariableScope::Global => engine
            .get_task_service()
            .set_task_variables(task.id.clone(), variables)?,
    }
    Ok(Json(
        updated_order
            .into_iter()
            .map(|(name, value)| scoped_variable_response(name, value, scope))
            .collect(),
    ))
}

/// Java PUT: update-only on the resolved scope (404 when the variable is
/// absent there). Accepts a JSON body or multipart/form-data (Java
/// `setBinaryVariable`); a body name must match the path name.
async fn update_task_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, variable_name)): Path<(String, String)>,
    request: Request,
) -> Result<Json<RestVariableResponse>, ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    let query_scope = requested_variable_scope(request.uri())?;
    let (value, scope) = if is_multipart(&request) {
        let multipart = Multipart::from_request(request, &())
            .await
            .map_err(multipart_error)?;
        let form = parse_binary_variable_form(multipart).await?;
        let (name, value, form_scope) = binary_variable_from_form(form)?;
        // Java: the multipart variable name must equal the path name.
        if name != variable_name {
            return Err(ApiError::bad_request(
                "Variable name in the body should be equal to the name used in the requested URL.",
            ));
        }
        (value, resolve_write_scope(form_scope.as_deref(), query_scope)?)
    } else {
        let body = request_body_string(request).await?;
        let request: VariableRequest = serde_json::from_str(&body)
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
        if let Some(name) = request.name.as_deref()
            && name != variable_name
        {
            return Err(ApiError::bad_request(
                "Variable name in the body should be equal to the name used in the requested URL.",
            ));
        }
        let scope = resolve_write_scope(request.scope.as_deref(), query_scope)?;
        let value = storage_value_for_data_backed_variable_request(&request)?;
        (value, scope)
    };
    engine.get_task_service().update_task_variable(
        task.id.clone(),
        scope,
        variable_name.clone(),
        value.clone(),
    )?;
    Ok(Json(scoped_variable_response(variable_name, value, scope)))
}

/// Java DELETE: removes the variable from the resolved scope, which defaults
/// to local; `?scope=` is the fallback. 404 when absent on that scope.
async fn delete_task_variable(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, variable_name)): Path<(String, String)>,
    uri: Uri,
) -> Result<StatusCode, ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    let scope = requested_variable_scope(&uri)?.unwrap_or(TaskVariableScope::Local);
    engine
        .get_task_service()
        .remove_task_variable_on_scope(task.id.clone(), scope, variable_name)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java DELETE on the collection: removes ALL task-local variables; global
/// variables are left untouched.
async fn delete_all_local_task_variables(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    engine
        .get_task_service()
        .remove_all_task_local_variables(task.id.clone())?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_task_variable_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, variable_name)): Path<(String, String)>,
    uri: Uri,
) -> Result<Response, ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    let (value, _) = find_task_variable_value(
        &engine,
        &task,
        &variable_name,
        requested_variable_scope(&uri)?,
    )?;
    variable_data_response(value)
}

/// Rust extension (raw-bytes data write, no Java counterpart): the variable
/// must already exist and be data-backed. Scope behavior is unchanged from
/// before the Java-parity rebuild: explicit `?scope=local` writes the local
/// variable, anything else writes the execution variable.
async fn update_task_variable_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((id, variable_name)): Path<(String, String)>,
    uri: Uri,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let task = super::tasks::load_task(&engine, &id)?;
    let scope = requested_variable_scope(&uri)?;
    if scope == Some(TaskVariableScope::Local) {
        let value = engine
            .get_task_service()
            .get_task_local_variable(task.id.clone(), variable_name.to_string())?
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Task '{}' local variable '{}' was not found",
                    task.id, variable_name
                ))
            })?;
        let variable_type = variable_data_type(&value).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Task '{}' local variable '{}' is not a binary, bytes, or serializable variable",
                task.id, variable_name
            ))
        })?;
        engine.get_task_service().set_task_local_variable(
            task.id.clone(),
            variable_name.to_string(),
            encode_variable_data(&variable_type, &body),
        )?;
        return Ok(StatusCode::NO_CONTENT);
    }
    super::process_instances::set_variable_data_for_execution(
        &engine,
        &task.execution_id,
        &variable_name,
        body,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

fn is_multipart(request: &Request) -> bool {
    request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().starts_with("multipart/form-data"))
        .unwrap_or(false)
}

async fn request_body_string(request: Request) -> Result<String, ApiError> {
    let bytes = axum::body::to_bytes(request.into_body(), MAX_REQUEST_BODY_BYTES)
        .await
        .map_err(|err| {
            let message = err.to_string();
            if message.contains("length limit exceeded") {
                ApiError::payload_too_large(format!(
                    "request body exceeds limit of {MAX_REQUEST_BODY_BYTES} bytes"
                ))
            } else {
                ApiError::bad_request(message)
            }
        })?;
    String::from_utf8(bytes.to_vec()).map_err(|err| ApiError::bad_request(err.to_string()))
}

fn multipart_error(err: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(err.to_string())
}

/// Stream a multipart field with per-field and request-total caps.
async fn read_multipart_field_limited(
    mut field: axum::extract::multipart::Field<'_>,
    per_field_limit: usize,
    request_total: &mut usize,
    request_limit: usize,
) -> Result<Vec<u8>, ApiError> {
    let mut buf = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        let n = chunk.len();
        if buf.len().saturating_add(n) > per_field_limit {
            return Err(ApiError::payload_too_large(format!(
                "multipart field exceeds limit of {per_field_limit} bytes"
            )));
        }
        if request_total.saturating_add(n) > request_limit {
            return Err(ApiError::payload_too_large(format!(
                "multipart request exceeds limit of {request_limit} bytes"
            )));
        }
        *request_total += n;
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Multipart form fields for Java `setBinaryVariable`: regular form fields
/// carry name/type/scope (matched case-insensitively like the Java parameter
/// loop); the first uploaded file provides the variable bytes.
#[derive(Default)]
struct BinaryVariableForm {
    name: Option<String>,
    variable_type: Option<String>,
    scope: Option<String>,
    file_bytes: Option<Vec<u8>>,
}

async fn parse_binary_variable_form(
    mut multipart: Multipart,
) -> Result<BinaryVariableForm, ApiError> {
    let mut form = BinaryVariableForm::default();
    let mut request_total = 0usize;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        if field.file_name().is_some() {
            // Java: use the first file in the request, ignore possible others.
            if form.file_bytes.is_none() {
                form.file_bytes = Some(
                    read_multipart_field_limited(
                        field,
                        MAX_MULTIPART_FILE_BYTES,
                        &mut request_total,
                        MAX_MULTIPART_REQUEST_BYTES,
                    )
                    .await?,
                );
            } else {
                let _ = read_multipart_field_limited(
                    field,
                    MAX_MULTIPART_FILE_BYTES,
                    &mut request_total,
                    MAX_MULTIPART_REQUEST_BYTES,
                )
                .await?;
            }
            continue;
        }
        let field_name = field.name().unwrap_or_default().to_string();
        let text_bytes = read_multipart_field_limited(
            field,
            MAX_MULTIPART_TEXT_FIELD_BYTES,
            &mut request_total,
            MAX_MULTIPART_REQUEST_BYTES,
        )
        .await?;
        let text = String::from_utf8(text_bytes)
            .map_err(|err| ApiError::bad_request(err.to_string()))?;
        if field_name.eq_ignore_ascii_case("scope") {
            form.scope = Some(text);
        } else if field_name.eq_ignore_ascii_case("name") {
            form.name = Some(text);
        } else if field_name.eq_ignore_ascii_case("type") {
            form.variable_type = Some(text);
        }
    }
    Ok(form)
}

/// Validates a parsed multipart form with the Java `setBinaryVariable`
/// messages and encodes the file bytes with the binary-variable marker so
/// they round-trip through GET .../data unchanged (serializable bytes stay
/// opaque; they are never deserialized).
fn binary_variable_from_form(
    form: BinaryVariableForm,
) -> Result<(String, Value, Option<String>), ApiError> {
    let bytes = form.file_bytes.ok_or_else(|| {
        ApiError::bad_request("No file content was found in request body.")
    })?;
    let name = form.name.ok_or_else(|| {
        ApiError::bad_request("No variable name was found in request body.")
    })?;
    let variable_type = match form.variable_type.as_deref() {
        // Java: an omitted type defaults to binary.
        None => "binary",
        Some("binary") => "binary",
        Some("serializable") => "serializable",
        Some(_) => {
            return Err(ApiError::bad_request(
                "Only 'binary' and 'serializable' are supported as variable type.",
            ));
        }
    };
    Ok((
        name,
        encode_variable_data(variable_type, &bytes),
        form.scope,
    ))
}
