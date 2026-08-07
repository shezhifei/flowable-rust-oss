use crate::error::ApiError;
use axum::extract::Query as AxumQuery;
use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode, header},
    routing::{delete, get, post},
};
use base64::Engine as _;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::IdentityLink;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type EngineState = Extension<Arc<ProcessEngine>>;

const PROCESS_INSTANCE_IDENTITY_LINKS_PATH: &str =
    "/runtime/process-instances/:process_instance_id/identity-links";
const PROCESS_INSTANCE_IDENTITY_LINK_PATH: &str =
    "/runtime/process-instances/:process_instance_id/identity-links/users/:identity_id/:link_type";
const ALT_PROCESS_INSTANCE_IDENTITY_LINKS_PATH: &str =
    "/runtime/process-instances/:process_instance_id/identitylinks";
const ALT_PROCESS_INSTANCE_IDENTITY_LINK_PATH: &str =
    "/runtime/process-instances/:process_instance_id/identitylinks/users/:identity_id/:link_type";
const TASK_IDENTITY_LINKS_PATH: &str = "/runtime/tasks/:task_id/identity-links";
const TASK_IDENTITY_LINKS_FAMILY_PATH: &str = "/runtime/tasks/:task_id/identity-links/:family";
const TASK_IDENTITY_LINK_PATH: &str =
    "/runtime/tasks/:task_id/identity-links/:family/:identity_id/:link_type";
const ALT_TASK_IDENTITY_LINKS_PATH: &str = "/runtime/tasks/:task_id/identitylinks";
const ALT_TASK_IDENTITY_LINKS_FAMILY_PATH: &str = "/runtime/tasks/:task_id/identitylinks/:family";
const ALT_TASK_IDENTITY_LINK_PATH: &str =
    "/runtime/tasks/:task_id/identitylinks/:family/:identity_id/:link_type";
const IDENTITY_LINKS_PATH: &str = "/runtime/identity-links";
const IDENTITY_LINK_PATH: &str = "/runtime/identity-links/:link_id";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_IDENTITY_LINKS_PATH}"),
            get(list_process_instance_identity_links).post(create_process_instance_identity_link),
        )
        .route(
            &format!("{prefix}{PROCESS_INSTANCE_IDENTITY_LINK_PATH}"),
            get(get_process_instance_identity_link).delete(delete_process_instance_identity_link),
        )
        .route(
            &format!("{prefix}{ALT_PROCESS_INSTANCE_IDENTITY_LINKS_PATH}"),
            get(list_process_instance_identity_links).post(create_process_instance_identity_link),
        )
        .route(
            &format!("{prefix}{ALT_PROCESS_INSTANCE_IDENTITY_LINK_PATH}"),
            get(get_process_instance_identity_link).delete(delete_process_instance_identity_link),
        )
        .route(
            &format!("{prefix}{TASK_IDENTITY_LINKS_PATH}"),
            get(list_task_identity_links).post(create_task_identity_link),
        )
        .route(
            &format!("{prefix}{TASK_IDENTITY_LINKS_FAMILY_PATH}"),
            get(list_task_identity_links_by_family),
        )
        .route(
            &format!("{prefix}{TASK_IDENTITY_LINK_PATH}"),
            get(get_task_identity_link).delete(delete_task_identity_link),
        )
        .route(
            &format!("{prefix}{ALT_TASK_IDENTITY_LINKS_PATH}"),
            get(list_task_identity_links).post(create_task_identity_link),
        )
        .route(
            &format!("{prefix}{ALT_TASK_IDENTITY_LINKS_FAMILY_PATH}"),
            get(list_task_identity_links_by_family),
        )
        .route(
            &format!("{prefix}{ALT_TASK_IDENTITY_LINK_PATH}"),
            get(get_task_identity_link).delete(delete_task_identity_link),
        )
        .route(
            &format!("{prefix}{IDENTITY_LINKS_PATH}"),
            post(create_identity_link).get(list_identity_links),
        )
        .route(
            &format!("{prefix}{IDENTITY_LINK_PATH}"),
            delete(delete_identity_link),
        )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityLinkResponse {
    pub id: String,
    pub link_type: String,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub process_definition_id: Option<String>,
}

impl From<IdentityLink> for IdentityLinkResponse {
    fn from(l: IdentityLink) -> Self {
        Self {
            id: l.id,
            link_type: l.link_type,
            user_id: l.user_id,
            group_id: l.group_id,
            task_id: l.task_id,
            process_instance_id: l.process_instance_id,
            process_definition_id: l.process_definition_id,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityLinkQueryParams {
    #[serde(alias = "task_id")]
    pub task_id: Option<String>,
    #[serde(alias = "process_instance_id")]
    pub process_instance_id: Option<String>,
    #[serde(alias = "process_definition_id")]
    pub process_definition_id: Option<String>,
    #[serde(alias = "user_id")]
    pub user_id: Option<String>,
    #[serde(alias = "group_id")]
    pub group_id: Option<String>,
    #[serde(alias = "link_type")]
    pub link_type: Option<String>,
}

pub async fn list_identity_links(
    engine: EngineState,
    params: AxumQuery<IdentityLinkQueryParams>,
) -> Result<Json<Vec<IdentityLinkResponse>>, ApiError> {
    let service = engine.0.get_identity_link_service();
    let mut query = service.create_identity_link_query();
    if let Some(task_id) = &params.task_id {
        query = query.task_id(task_id.clone());
    }
    if let Some(process_instance_id) = &params.process_instance_id {
        query = query.process_instance_id(process_instance_id.clone());
    }
    if let Some(process_definition_id) = &params.process_definition_id {
        query = query.process_definition_id(process_definition_id.clone());
    }
    if let Some(user_id) = &params.user_id {
        query = query.user_id(user_id.clone());
    }
    if let Some(group_id) = &params.group_id {
        query = query.group_id(group_id.clone());
    }
    if let Some(link_type) = &params.link_type {
        query = query.link_type(link_type.clone());
    }
    let mut links = query
        .list()
        .map_err(|e| ApiError::InternalServerError(format!("Identity link query failed: {}", e)))?;
    retain_identity_link_query_params(&mut links, &params);
    Ok(Json(
        links.into_iter().map(IdentityLinkResponse::from).collect(),
    ))
}

#[derive(Serialize)]
pub struct RestIdentityLinkResponse {
    pub url: Option<String>,
    pub user: Option<String>,
    pub group: Option<String>,
    #[serde(rename = "type")]
    pub link_type: String,
}

impl From<IdentityLink> for RestIdentityLinkResponse {
    fn from(link: IdentityLink) -> Self {
        let url = rest_identity_link_url(&link);
        Self {
            url,
            user: link.user_id,
            group: link.group_id,
            link_type: link.link_type,
        }
    }
}

pub async fn list_process_instance_identity_links(
    engine: EngineState,
    Path(process_instance_id): Path<String>,
) -> Result<Json<Vec<RestIdentityLinkResponse>>, ApiError> {
    ensure_process_instance_exists(&engine.0, &process_instance_id)?;

    let links = engine
        .0
        .get_identity_link_service()
        .create_identity_link_query()
        .process_instance_id(process_instance_id)
        .list()
        .map_err(|e| ApiError::InternalServerError(format!("Identity link query failed: {}", e)))?;

    Ok(Json(
        links
            .into_iter()
            .map(RestIdentityLinkResponse::from)
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct CreateProcessInstanceIdentityLinkRequest {
    pub user: Option<String>,
    pub group: Option<String>,
    #[serde(rename = "type")]
    pub link_type: Option<String>,
}

pub async fn create_process_instance_identity_link(
    engine: EngineState,
    Path(process_instance_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateProcessInstanceIdentityLinkRequest>,
) -> Result<(StatusCode, Json<RestIdentityLinkResponse>), ApiError> {
    ensure_process_instance_exists(&engine.0, &process_instance_id)?;

    if req.group.is_some() {
        return Err(ApiError::BadRequest(
            "Only user identity links are supported on a process instance.".to_string(),
        ));
    }
    let user = req
        .user
        .ok_or_else(|| ApiError::BadRequest("The user is required.".to_string()))?;
    let link_type = req
        .link_type
        .ok_or_else(|| ApiError::BadRequest("The identity link type is required.".to_string()))?;

    // Java `IdentityLinkEntityManagerImpl` performs no dedup on the
    // process-instance create path: each POST inserts a fresh row (a duplicate
    // user+type POST yields two collection entries). Use a random id so repeat
    // POSTs append rather than upsert onto a deterministic key.
    let link = IdentityLink {
        id: uuid::Uuid::new_v4().to_string(),
        link_type,
        user_id: Some(user),
        group_id: None,
        task_id: None,
        process_instance_id: Some(process_instance_id),
        process_definition_id: None,
    };

    // Java threads the authenticated principal onto the identity-link comment
    // (`createProcessInstanceIdentityLinkComment`); use the request principal.
    let author = user_id_from_basic_auth(&headers);
    engine
        .0
        .get_identity_link_service()
        .add_identity_link_with_author(link.clone(), author);

    Ok((
        StatusCode::CREATED,
        Json(RestIdentityLinkResponse::from(link)),
    ))
}

pub async fn get_process_instance_identity_link(
    engine: EngineState,
    Path((process_instance_id, identity_id, link_type)): Path<(String, String, String)>,
) -> Result<Json<RestIdentityLinkResponse>, ApiError> {
    ensure_process_instance_exists(&engine.0, &process_instance_id)?;
    let link =
        process_instance_identity_link(&engine.0, &process_instance_id, &identity_id, &link_type)?;
    Ok(Json(RestIdentityLinkResponse::from(link)))
}

pub async fn delete_process_instance_identity_link(
    engine: EngineState,
    Path((process_instance_id, identity_id, link_type)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    ensure_process_instance_exists(&engine.0, &process_instance_id)?;
    let link =
        process_instance_identity_link(&engine.0, &process_instance_id, &identity_id, &link_type)?;
    let author = user_id_from_basic_auth(&headers);
    engine
        .0
        .get_identity_link_service()
        .remove_identity_link_with_author(&link.id, author);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateTaskIdentityLinkRequest {
    #[serde(alias = "userId", alias = "user_id")]
    pub user: Option<String>,
    #[serde(alias = "groupId", alias = "group_id")]
    pub group: Option<String>,
    #[serde(rename = "type", alias = "linkType", alias = "link_type")]
    pub link_type: Option<String>,
}

pub async fn list_task_identity_links(
    engine: EngineState,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<RestIdentityLinkResponse>>, ApiError> {
    ensure_task_exists(&engine.0, &task_id)?;
    Ok(Json(
        task_identity_links(&engine.0, &task_id)?
            .into_iter()
            .map(RestIdentityLinkResponse::from)
            .collect(),
    ))
}

pub async fn create_task_identity_link(
    engine: EngineState,
    Path(task_id): Path<String>,
    Json(req): Json<CreateTaskIdentityLinkRequest>,
) -> Result<(StatusCode, Json<RestIdentityLinkResponse>), ApiError> {
    ensure_task_active(&engine.0, &task_id)?;

    let link_type = req
        .link_type
        .ok_or_else(|| ApiError::BadRequest("The identity link type is required.".to_string()))?;

    let (user_id, group_id, family, identity_id) = match (req.user, req.group) {
        (Some(user), None) => (Some(user.clone()), None, "users", user),
        (None, Some(group)) => (None, Some(group.clone()), "groups", group),
        (None, None) => {
            return Err(ApiError::BadRequest(
                "A group or a user is required to create an identity link.".to_string(),
            ));
        }
        (Some(_), Some(_)) => {
            return Err(ApiError::BadRequest(
                "Only one of user or group can be used to create an identity link.".to_string(),
            ));
        }
    };

    let link = IdentityLink {
        id: task_identity_link_id(&task_id, family, &identity_id, &link_type),
        link_type: link_type.clone(),
        user_id: user_id.clone(),
        group_id: group_id.clone(),
        task_id: Some(task_id.clone()),
        process_instance_id: None,
        process_definition_id: None,
    };

    if link_type == "assignee" || link_type == "owner" {
        if group_id.is_some() {
            return Err(ApiError::BadRequest(
                "Assignee and owner identity links require a user.".to_string(),
            ));
        }
        engine
            .0
            .get_task_service()
            .add_identity_link(task_id, user_id, None, link_type)
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    } else {
        engine
            .0
            .get_identity_link_service()
            .add_identity_link(link.clone());
    }

    Ok((
        StatusCode::CREATED,
        Json(RestIdentityLinkResponse::from(link)),
    ))
}

pub async fn list_task_identity_links_by_family(
    engine: EngineState,
    Path((task_id, family)): Path<(String, String)>,
) -> Result<Json<Vec<RestIdentityLinkResponse>>, ApiError> {
    ensure_task_exists(&engine.0, &task_id)?;
    let family = normalize_identity_link_family(&family)?;
    let mut links = task_identity_links(&engine.0, &task_id)?;
    retain_identity_link_family(&mut links, family);
    Ok(Json(
        links
            .into_iter()
            .map(RestIdentityLinkResponse::from)
            .collect(),
    ))
}

pub async fn get_task_identity_link(
    engine: EngineState,
    Path((task_id, family, identity_id, link_type)): Path<(String, String, String, String)>,
) -> Result<Json<RestIdentityLinkResponse>, ApiError> {
    ensure_task_exists(&engine.0, &task_id)?;
    let family = normalize_identity_link_family(&family)?;
    let link = task_identity_link(&engine.0, &task_id, family, &identity_id, &link_type)?;
    Ok(Json(RestIdentityLinkResponse::from(link)))
}

pub async fn delete_task_identity_link(
    engine: EngineState,
    Path((task_id, family, identity_id, link_type)): Path<(String, String, String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_task_active(&engine.0, &task_id)?;
    let family = normalize_identity_link_family(&family)?;
    if link_type == "assignee" || link_type == "owner" {
        if family != "users" {
            return Err(ApiError::BadRequest(
                "Assignee and owner identity links require a user.".to_string(),
            ));
        }
        engine
            .0
            .get_task_service()
            .delete_identity_link(task_id, Some(identity_id), None, link_type)
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let link = task_identity_link(&engine.0, &task_id, family, &identity_id, &link_type)?;
    engine
        .0
        .get_identity_link_service()
        .remove_identity_link(&link.id);
    Ok(StatusCode::NO_CONTENT)
}

fn ensure_process_instance_exists(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Result<(), ApiError> {
    // Java `BaseProcessInstanceResource.getProcessInstanceFromRequest` queries the
    // *runtime* process instance only (RuntimeService.createProcessInstanceQuery),
    // so a completed/ended instance is a 404 for the runtime identity-link family.
    // (History identity-link endpoints query historic instances and remain
    // reachable after completion — see routes/history.rs.)
    engine
        .get_runtime_store()
        .db_store()
        .find_by_id::<flowable_engine::runtime::process_instance::ProcessInstance>(
            "process_instances",
            process_instance_id,
        )
        .unwrap()
        .filter(|process_instance| !process_instance.is_ended)
        .map(|_| ())
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Process instance '{}' was not found",
                process_instance_id
            ))
        })
}

/// Extracts the request principal from an HTTP Basic `Authorization` header.
/// Java sets the identity-link comment author from
/// `Authentication.getAuthenticatedUserId()`; the REST layer authenticates via
/// Basic auth, so the username is the principal. Mirrors
/// `routes::tasks::user_id_from_basic_auth`.
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

fn ensure_task_exists(engine: &ProcessEngine, task_id: &str) -> Result<(), ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_runtime_store()
        .find_task(task_id, &mut session)
        .map(|_| ())
        .ok_or_else(|| ApiError::NotFound(format!("Task '{}' was not found", task_id)))
}

/// Java parity: `AddIdentityLinkCmd` / `DeleteIdentityLinkCmd` extend
/// `NeedsActiveTaskCmd` — loading the task AND rejecting suspended tasks.
fn ensure_task_active(engine: &ProcessEngine, task_id: &str) -> Result<(), ApiError> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let task = engine
        .get_runtime_store()
        .find_task(task_id, &mut session)
        .ok_or_else(|| ApiError::NotFound(format!("Task '{}' was not found", task_id)))?;
    if task.is_suspended() {
        return Err(ApiError::InternalServerError(format!(
            "Cannot execute operation for a suspended task '{}'",
            task_id
        )));
    }
    Ok(())
}

fn task_identity_links(
    engine: &ProcessEngine,
    task_id: &str,
) -> Result<Vec<IdentityLink>, ApiError> {
    engine
        .get_identity_link_service()
        .create_identity_link_query()
        .task_id(task_id.to_string())
        .list()
        .map_err(|e| ApiError::InternalServerError(format!("Identity link query failed: {}", e)))
}

fn process_instance_identity_link(
    engine: &ProcessEngine,
    process_instance_id: &str,
    identity_id: &str,
    link_type: &str,
) -> Result<IdentityLink, ApiError> {
    engine
        .get_identity_link_service()
        .create_identity_link_query()
        .process_instance_id(process_instance_id.to_string())
        .user_id(identity_id.to_string())
        .link_type(link_type.to_string())
        .list()
        .map_err(|e| ApiError::InternalServerError(format!("Identity link query failed: {}", e)))?
        .into_iter()
        .find(|link| link.group_id.is_none() && link.link_type == link_type)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Identity link '{}' was not found",
                process_instance_identity_link_id(process_instance_id, identity_id, link_type)
            ))
        })
}

fn task_identity_link(
    engine: &ProcessEngine,
    task_id: &str,
    family: &str,
    identity_id: &str,
    link_type: &str,
) -> Result<IdentityLink, ApiError> {
    let family = normalize_identity_link_family(family)?;
    task_identity_links(engine, task_id)?
        .into_iter()
        .find(|link| {
            link.link_type == link_type
                && match family {
                    "users" => link.user_id.as_deref() == Some(identity_id),
                    "groups" => link.group_id.as_deref() == Some(identity_id),
                    _ => false,
                }
        })
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Identity link '{}' was not found",
                task_identity_link_id(task_id, family, identity_id, link_type)
            ))
        })
}

fn normalize_identity_link_family(family: &str) -> Result<&'static str, ApiError> {
    if family.eq_ignore_ascii_case("users") {
        Ok("users")
    } else if family.eq_ignore_ascii_case("groups") {
        Ok("groups")
    } else {
        Err(ApiError::BadRequest(format!(
            "Unsupported identity link family '{}'",
            family
        )))
    }
}

fn retain_identity_link_family(links: &mut Vec<IdentityLink>, family: &str) {
    match family {
        "users" => links.retain(|link| link.user_id.is_some()),
        "groups" => links.retain(|link| link.group_id.is_some()),
        _ => {}
    }
}

fn retain_identity_link_query_params(
    links: &mut Vec<IdentityLink>,
    params: &IdentityLinkQueryParams,
) {
    links.retain(|link| {
        params
            .task_id
            .as_deref()
            .is_none_or(|task_id| link.task_id.as_deref() == Some(task_id))
            && params
                .process_instance_id
                .as_deref()
                .is_none_or(|process_instance_id| {
                    link.process_instance_id.as_deref() == Some(process_instance_id)
                })
            && params
                .process_definition_id
                .as_deref()
                .is_none_or(|process_definition_id| {
                    link.process_definition_id.as_deref() == Some(process_definition_id)
                })
            && params
                .user_id
                .as_deref()
                .is_none_or(|user_id| link.user_id.as_deref() == Some(user_id))
            && params
                .group_id
                .as_deref()
                .is_none_or(|group_id| link.group_id.as_deref() == Some(group_id))
            && params
                .link_type
                .as_deref()
                .is_none_or(|link_type| link.link_type == link_type)
    });
}

fn identity_link_family_and_id(link: &IdentityLink) -> Option<(String, String)> {
    if let Some(user_id) = &link.user_id {
        Some(("users".to_string(), user_id.clone()))
    } else {
        link.group_id
            .as_ref()
            .map(|group_id| ("groups".to_string(), group_id.clone()))
    }
}

fn task_identity_link_id(
    task_id: &str,
    family: &str,
    identity_id: &str,
    link_type: &str,
) -> String {
    format!("task:{task_id}:{family}:{identity_id}:type:{link_type}")
}

fn process_instance_identity_link_id(
    process_instance_id: &str,
    identity_id: &str,
    link_type: &str,
) -> String {
    format!("process-instance:{process_instance_id}:user:{identity_id}:type:{link_type}")
}

fn rest_identity_link_url(link: &IdentityLink) -> Option<String> {
    let (family, identity_id) = identity_link_family_and_id(link)?;
    if let Some(process_definition_id) = &link.process_definition_id {
        Some(format!(
            "/repository/process-definitions/{process_definition_id}/identitylinks/{family}/{identity_id}"
        ))
    } else if let Some(task_id) = &link.task_id {
        Some(format!(
            "/runtime/tasks/{task_id}/identitylinks/{family}/{identity_id}/{}",
            link.link_type
        ))
    } else {
        link.process_instance_id.as_ref().map(|process_instance_id| {
            format!(
                "/runtime/process-instances/{process_instance_id}/identitylinks/users/{identity_id}/{}",
                link.link_type
            )
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIdentityLinkRequest {
    pub id: String,
    #[serde(alias = "link_type")]
    pub link_type: String,
    #[serde(alias = "user_id")]
    pub user_id: Option<String>,
    #[serde(alias = "group_id")]
    pub group_id: Option<String>,
    #[serde(alias = "task_id")]
    pub task_id: Option<String>,
    #[serde(alias = "process_instance_id")]
    pub process_instance_id: Option<String>,
    #[serde(alias = "process_definition_id")]
    pub process_definition_id: Option<String>,
}

pub async fn create_identity_link(
    engine: EngineState,
    Json(req): Json<CreateIdentityLinkRequest>,
) -> Result<Json<IdentityLinkResponse>, ApiError> {
    let link = IdentityLink {
        id: req.id.clone(),
        link_type: req.link_type.clone(),
        user_id: req.user_id.clone(),
        group_id: req.group_id.clone(),
        task_id: req.task_id.clone(),
        process_instance_id: req.process_instance_id.clone(),
        process_definition_id: req.process_definition_id.clone(),
    };
    engine
        .0
        .get_identity_link_service()
        .add_identity_link(link.clone());
    Ok(Json(IdentityLinkResponse::from(link)))
}

pub async fn delete_identity_link(
    engine: EngineState,
    Path(link_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let service = engine.0.get_identity_link_service();
    let exists = service
        .create_identity_link_query()
        .list()
        .map_err(|e| ApiError::InternalServerError(format!("Identity link query failed: {}", e)))?
        .into_iter()
        .any(|link| link.id == link_id);

    if !exists {
        return Err(ApiError::NotFound(format!(
            "Identity link '{}' was not found",
            link_id
        )));
    }

    service.remove_identity_link(&link_id);
    Ok(StatusCode::NO_CONTENT)
}
